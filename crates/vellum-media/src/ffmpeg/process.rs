//! running ffmpeg and ffprobe as child processes.
//!
//! stdout is always read on the calling thread, so cancel is checked between reads and a
//! paused job simply stops reading, which stalls ffmpeg once the pipe fills. stderr is
//! drained on a helper thread so a chatty process can never deadlock on a full pipe, and
//! its last lines become the error message when something fails.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use std::sync::Arc;

use crate::control::Control;
use crate::error::{MediaError, MediaResult};

/// stderr lines kept for error messages
const STDERR_LINES: usize = 12;

/// builds a command that never opens a console window on windows
pub fn command(program: &std::path::Path) -> Command {
    #[allow(unused_mut)] // only mutated on windows
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// a running child that is killed if it is dropped before it finishes, so a cancelled or
/// panicking job never leaves ffmpeg running in the background
pub struct Running {
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<JoinHandle<()>>,
    tail: Arc<Mutex<VecDeque<String>>>,
    finished: bool,
}

impl Running {
    pub fn spawn(mut cmd: Command) -> MediaResult<Self> {
        let program = cmd.get_program().to_owned();
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|err| MediaError::io(&program, err))?;
        let stdout = child.stdout.take();
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let stderr = child.stderr.take().and_then(|stderr| {
            let tail = tail.clone();
            thread::Builder::new()
                .name("vellum-ffmpeg-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines() {
                        let Ok(line) = line else { break };
                        let line = line.trim().to_owned();
                        if line.is_empty() {
                            continue;
                        }
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
            stdout,
            stderr,
            tail,
            finished: false,
        })
    }

    /// reads stdout in chunks until it ends. `on_chunk` runs on this thread and may stop
    /// early by returning an error, which also kills the child.
    pub fn stream(
        &mut self,
        control: &dyn Control,
        mut on_chunk: impl FnMut(&[u8]) -> MediaResult<()>,
    ) -> MediaResult<()> {
        let Some(mut stdout) = self.stdout.take() else {
            return Ok(());
        };
        let mut buf = vec![0_u8; 64 * 1024];
        loop {
            if let Err(err) = control.checkpoint() {
                self.kill();
                return Err(err);
            }
            let read = match stdout.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    self.kill();
                    return Err(MediaError::Ffmpeg(err.to_string()));
                }
            };
            if let Err(err) = on_chunk(&buf[..read]) {
                self.kill();
                return Err(err);
            }
        }
    }

    /// reads stdout line by line, for `-progress pipe:1`
    pub fn lines(
        &mut self,
        control: &dyn Control,
        mut on_line: impl FnMut(&str),
    ) -> MediaResult<()> {
        let Some(stdout) = self.stdout.take() else {
            return Ok(());
        };
        for line in BufReader::new(stdout).lines() {
            if let Err(err) = control.checkpoint() {
                self.kill();
                return Err(err);
            }
            match line {
                Ok(line) => on_line(&line),
                Err(_) => break,
            }
        }
        // one more check so a cancel during the last stretch is not reported as success
        if let Err(err) = control.checkpoint() {
            self.kill();
            return Err(err);
        }
        Ok(())
    }

    /// waits for the child and turns a failure into an error built from stderr
    pub fn finish(mut self) -> MediaResult<()> {
        let status = self.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(MediaError::Ffmpeg(self.error_text(status)))
        }
    }

    fn wait(&mut self) -> MediaResult<ExitStatus> {
        // drop our end of stdout first so a child still writing gets a broken pipe
        self.stdout = None;
        let status = self
            .child
            .wait()
            .map_err(|err| MediaError::Ffmpeg(err.to_string()))?;
        self.finished = true;
        if let Some(handle) = self.stderr.take() {
            let _ = handle.join();
        }
        Ok(status)
    }

    fn error_text(&self, status: ExitStatus) -> String {
        let tail = self.tail.lock();
        let useful: Vec<&str> = tail
            .iter()
            .map(String::as_str)
            .filter(|line| !line.starts_with("Conversion failed"))
            .collect();
        match useful.last() {
            Some(last) => (*last).to_owned(),
            None => format!("it stopped with {status}"),
        }
    }

    fn kill(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.wait();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.kill();
    }
}

/// runs a short command and collects all of stdout, killing it after `timeout`. for
/// ffprobe and capability checks, not for encodes.
pub fn capture(cmd: Command, timeout: Duration) -> MediaResult<Vec<u8>> {
    let mut running = Running::spawn(cmd)?;
    let deadline = Instant::now() + timeout;
    let collected = Arc::new(Mutex::new(Vec::new()));
    let reader = running.stdout.take().and_then(|mut stdout| {
        let collected = collected.clone();
        thread::Builder::new()
            .name("vellum-ffmpeg-stdout".into())
            .spawn(move || {
                let mut buf = Vec::new();
                let _ = stdout.read_to_end(&mut buf);
                *collected.lock() = buf;
            })
            .ok()
    });
    loop {
        match running.child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                running.kill();
                return Err(MediaError::Ffmpeg(format!(
                    "it took longer than {} seconds and was stopped",
                    timeout.as_secs()
                )));
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(err) => return Err(MediaError::Ffmpeg(err.to_string())),
        }
    }
    if let Some(reader) = reader {
        let _ = reader.join();
    }
    running.finish()?;
    let out = std::mem::take(&mut *collected.lock());
    Ok(out)
}

/// turns ffmpeg `-progress` lines into a fraction of `duration`
#[derive(Debug, Clone, Copy)]
pub struct ProgressParser {
    duration_us: f64,
}

impl ProgressParser {
    pub fn new(duration_secs: Option<f64>) -> Option<Self> {
        let duration = duration_secs.filter(|d| d.is_finite() && *d > 0.0)?;
        Some(Self {
            duration_us: duration * 1_000_000.0,
        })
    }

    pub fn parse(&self, line: &str) -> Option<f32> {
        // out_time_ms is in microseconds too, a long standing ffmpeg quirk
        let value = line
            .strip_prefix("out_time_us=")
            .or_else(|| line.strip_prefix("out_time_ms="))?;
        let us: f64 = value.trim().parse().ok()?;
        if us < 0.0 {
            return None;
        }
        Some((us / self.duration_us).clamp(0.0, 1.0) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_become_fractions() {
        let parser = ProgressParser::new(Some(10.0)).unwrap();
        assert_eq!(parser.parse("out_time_us=5000000"), Some(0.5));
        assert_eq!(parser.parse("out_time_ms=20000000"), Some(1.0));
        assert_eq!(parser.parse("out_time_us=N/A"), None);
        assert_eq!(parser.parse("out_time_us=-5"), None);
        assert_eq!(parser.parse("frame=12"), None);
        assert!(ProgressParser::new(Some(0.0)).is_none());
        assert!(ProgressParser::new(None).is_none());
    }
}
