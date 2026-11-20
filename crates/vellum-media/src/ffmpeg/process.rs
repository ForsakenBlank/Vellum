//! running external tools so that cancel and pause reach them.
//!
//! a blocking ffmpeg pass can take minutes, so jobs cannot just call `checkpoint` between
//! frames the way in-process work does. instead the job thread watches the child process:
//! cancel kills it, and pause stops it with SIGSTOP on unix. windows has no safe way to
//! suspend another process from rust without ffi, so there a pause request is honoured
//! when the current pass ends.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::Mutex;
use vellum_core::{JobContext, JobError};

use crate::error::{MediaError, MediaResult};

/// how long running work hears about cancel and pause and reports progress. jobs pass
/// their `JobContext`; quick calls from the ui pass [`NoReporter`].
pub trait Reporter {
    /// true once the work should stop
    fn cancelled(&self) -> bool;
    /// true while the user wants the work paused
    fn pause_requested(&self) -> bool;
    /// blocks while paused and errors once cancelled
    fn checkpoint(&self) -> MediaResult<()>;
    /// 0 to 1
    fn progress(&self, fraction: f32);
    fn message(&self, text: &str);
}

impl Reporter for JobContext {
    fn cancelled(&self) -> bool {
        self.is_cancelled()
    }

    fn pause_requested(&self) -> bool {
        self.is_pause_requested()
    }

    fn checkpoint(&self) -> MediaResult<()> {
        JobContext::checkpoint(self).map_err(|err| match err {
            JobError::Cancelled => MediaError::Cancelled,
            JobError::Failed { message, .. } => MediaError::Decode(message),
        })
    }

    fn progress(&self, fraction: f32) {
        self.set_progress(fraction);
    }

    fn message(&self, text: &str) {
        self.set_message(text);
    }
}

/// for work that is never cancelled or paused, like a probe the ui waits on
pub struct NoReporter;

impl Reporter for NoReporter {
    fn cancelled(&self) -> bool {
        false
    }

    fn pause_requested(&self) -> bool {
        false
    }

    fn checkpoint(&self) -> MediaResult<()> {
        Ok(())
    }

    fn progress(&self, _fraction: f32) {}

    fn message(&self, _text: &str) {}
}

/// a command that never flashes a console window on windows and never reads the keyboard
pub(crate) fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// runs to the end and returns stdout. a failure carries the end of stderr.
pub(crate) fn run_output(cmd: &mut Command, tool: &str) -> MediaResult<Vec<u8>> {
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| spawn_error(tool, &err))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(MediaError::tool(tool, summarise(stderr.lines())))
}

/// runs a command and calls `line` for every line it writes to stdout, which is how
/// `ffmpeg -progress pipe:1` reports. cancel and pause are checked about ten times a second.
pub(crate) fn run_lines(
    cmd: &mut Command,
    tool: &str,
    reporter: &dyn Reporter,
    mut line: impl FnMut(&str),
) -> MediaResult<()> {
    let mut running = Running::spawn(cmd, tool)?;
    let stdout = running.take_stdout()?;
    let (tx, rx) = mpsc::channel::<String>();
    let reader = thread::Builder::new()
        .name(format!("{tool}-stdout"))
        .spawn(move || {
            for text in BufReader::new(stdout).lines() {
                let Ok(text) = text else { break };
                if tx.send(text).is_err() {
                    break;
                }
            }
        })
        .map_err(|err| spawn_error(tool, &err))?;

    let outcome = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(text) => line(&text),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break Ok(()),
        }
        if let Err(err) = running.honour(reporter) {
            break Err(err);
        }
    };
    let result = match outcome {
        Ok(()) => running.finish(),
        Err(err) => {
            running.kill();
            Err(err)
        }
    };
    let _ = reader.join();
    result
}

/// runs a command and hands its stdout to `read`, which pulls bytes until it has what it
/// needs and returns. `read` should call `reporter.checkpoint()` between chunks: while it
/// blocks there the pipe fills and the tool waits, which pauses it on every platform.
pub(crate) fn run_reading<T>(
    cmd: &mut Command,
    tool: &str,
    read: impl FnOnce(&mut ChildStdout) -> MediaResult<T>,
) -> MediaResult<T> {
    let mut running = Running::spawn(cmd, tool)?;
    let mut stdout = running.take_stdout()?;
    match read(&mut stdout) {
        Ok(value) => {
            // the reader may stop early (one frame is enough), so drain the rest to let
            // the tool exit instead of dying on a broken pipe
            let _ = std::io::copy(&mut stdout, &mut std::io::sink());
            drop(stdout);
            running.finish()?;
            Ok(value)
        }
        Err(err) => {
            running.kill();
            // a damaged file often shows up as a short read; stderr says why
            match err {
                MediaError::Cancelled => Err(MediaError::Cancelled),
                other => Err(running.stderr_error().unwrap_or(other)),
            }
        }
    }
}

/// reads exactly `buf.len()` bytes unless the stream ends first. returns the count read.
pub(crate) fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(filled)
}

struct Running {
    child: Child,
    tool: String,
    stderr: Arc<Mutex<VecDeque<String>>>,
    stderr_thread: Option<JoinHandle<()>>,
}

const STDERR_LINES: usize = 30;

impl Running {
    fn spawn(cmd: &mut Command, tool: &str) -> MediaResult<Self> {
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| spawn_error(tool, &err))?;
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_thread = child.stderr.take().and_then(|pipe| {
            let tail = stderr.clone();
            thread::Builder::new()
                .name(format!("{tool}-stderr"))
                .spawn(move || {
                    for line in BufReader::new(pipe).lines() {
                        let Ok(line) = line else { break };
                        let mut tail = tail.lock();
                        if tail.len() == STDERR_LINES {
                            tail.pop_front();
                        }
                        tail.push_back(line);
                    }
                })
                .ok()
        });
        Ok(Self {
            child,
            tool: tool.to_owned(),
            stderr,
            stderr_thread,
        })
    }

    fn take_stdout(&mut self) -> MediaResult<ChildStdout> {
        self.child
            .stdout
            .take()
            .ok_or_else(|| MediaError::tool(&self.tool, "its output could not be read"))
    }

    /// kills the child on cancel and stops it while paused
    fn honour(&mut self, reporter: &dyn Reporter) -> MediaResult<()> {
        if reporter.cancelled() {
            return Err(MediaError::Cancelled);
        }
        if reporter.pause_requested() && suspend::supported() {
            let pid = self.child.id();
            suspend::stop(pid);
            let outcome = reporter.checkpoint();
            suspend::resume(pid);
            outcome?;
        }
        Ok(())
    }

    fn finish(mut self) -> MediaResult<()> {
        let status = self
            .child
            .wait()
            .map_err(|err| MediaError::tool(&self.tool, err.to_string()))?;
        self.join_stderr();
        if status.success() {
            return Ok(());
        }
        Err(self
            .stderr_error()
            .unwrap_or_else(|| MediaError::tool(&self.tool, format!("it stopped with {status}"))))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.join_stderr();
    }

    fn join_stderr(&mut self) {
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }

    fn stderr_error(&mut self) -> Option<MediaError> {
        self.join_stderr();
        let tail = self.stderr.lock();
        if tail.is_empty() {
            return None;
        }
        Some(MediaError::tool(
            &self.tool,
            summarise(tail.iter().map(String::as_str)),
        ))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // never leave a tool running behind a job that gave up on it
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn spawn_error(tool: &str, err: &std::io::Error) -> MediaError {
    if err.kind() == std::io::ErrorKind::NotFound {
        return MediaError::FfmpegMissing;
    }
    MediaError::tool(tool, format!("it could not be started: {err}"))
}

/// the last few useful lines of a tool's error output, on one line
fn summarise<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let lines: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    let start = lines.len().saturating_sub(3);
    let text = lines[start..].join(" ");
    if text.is_empty() {
        "it gave no reason".to_owned()
    } else {
        text
    }
}

#[cfg(unix)]
mod suspend {
    use std::process::{Command, Stdio};

    pub fn supported() -> bool {
        true
    }

    fn signal(pid: u32, sig: &str) {
        let sent = Command::new("kill")
            .args([sig, &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Err(err) = sent {
            tracing::warn!(pid, sig, %err, "could not signal a child process");
        }
    }

    pub fn stop(pid: u32) {
        signal(pid, "-STOP");
    }

    pub fn resume(pid: u32) {
        signal(pid, "-CONT");
    }
}

#[cfg(not(unix))]
mod suspend {
    pub fn supported() -> bool {
        false
    }

    pub fn stop(_pid: u32) {}

    pub fn resume(_pid: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_keep_the_last_useful_lines() {
        let lines = ["", "a", "  ", "b", "c", "d"];
        assert_eq!(summarise(lines.into_iter()), "b c d");
        assert_eq!(summarise(std::iter::empty()), "it gave no reason");
    }

    #[test]
    fn a_missing_program_reads_as_missing_ffmpeg() {
        let mut cmd = command(Path::new("/definitely/not/here/ffmpeg"));
        assert!(matches!(
            run_output(&mut cmd, "ffmpeg"),
            Err(MediaError::FfmpegMissing)
        ));
    }
}
