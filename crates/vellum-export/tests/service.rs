//! the quick tools as the app runs them: a session document, its options changed by
//! commands, one job per file owned by the document, results the ui can open.

// test helpers unwrap too, a failure in one is a failed test
#![allow(clippy::unwrap_used)]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture;
use serde_json::json;
use vellum_core::app::{AppPaths, AppServices};
use vellum_core::command::CommandRecord;
use vellum_core::jobs::{JobId, JobState};
use vellum_export::anim::AnimOptions;
use vellum_export::service::QuickTools;
use vellum_export::size::SizeOptions;
use vellum_media::MediaService;

struct App {
    _dir: tempfile::TempDir,
    app: AppServices,
    tools: QuickTools,
}

fn app() -> Option<App> {
    let fx = fixture()?;
    let dir = tempfile::tempdir().unwrap();
    let app = AppServices::start(AppPaths {
        config_dir: dir.path().join("config"),
        data_dir: dir.path().join("data"),
    })
    .unwrap();
    let out = dir.path().join("results");
    app.settings
        .apply_patch(&json!({ "export": { "outputFolder": out.display().to_string() } }))
        .unwrap();
    let media = MediaService::new(
        Some(fx.ffmpeg.clone()),
        &dir.path().join("cache"),
        &app.settings.get().media,
        app.jobs.clone(),
        app.bus.clone(),
    );
    let tools = QuickTools::new(&app, &media);
    Some(App {
        _dir: dir,
        app,
        tools,
    })
}

fn open(app: &App, tool: &str, files: &[PathBuf]) -> String {
    let opened = app.app.open_files(files, Some(tool), "main");
    opened.opened[0].id.clone()
}

fn set(app: &App, id: &str, patch: serde_json::Value) {
    app.app
        .documents
        .execute(
            id,
            &CommandRecord {
                kind: "tool.set_options".into(),
                params: json!({ "patch": patch }),
            },
        )
        .unwrap();
}

fn wait(app: &App, ids: &[JobId]) -> Vec<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let snapshots: Vec<_> = ids
            .iter()
            .map(|id| app.app.jobs.snapshot(*id).unwrap())
            .collect();
        if snapshots.iter().all(|s| s.state.is_finished()) {
            return snapshots
                .into_iter()
                .map(|s| {
                    assert_eq!(s.state, JobState::Completed, "{}: {:?}", s.title, s.error);
                    s.result.unwrap()
                })
                .collect();
        }
        assert!(Instant::now() < deadline, "jobs did not finish");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn export_to_size_runs_one_owned_job_per_file() {
    let Some(app) = app() else { return };
    let fx = fixture().unwrap();
    let files = [fx.file("noisy.mp4"), fx.file("photo.png")];
    let id = open(&app, "export_to_size", &files);
    set(
        &app,
        &id,
        json!({ "target": { "amount": 300, "unit": "MB" } }),
    );
    // undo works on options like any other change
    app.app.documents.undo(&id).unwrap();
    set(
        &app,
        &id,
        json!({ "target": { "amount": 0.3, "unit": "MB" } }),
    );

    let jobs = app.tools.run(&id).unwrap();
    assert_eq!(jobs.len(), 2);
    let snapshot = app.app.jobs.snapshot(jobs[0]).unwrap();
    assert_eq!(snapshot.owner.as_deref(), Some(id.as_str()));
    assert_eq!(snapshot.title, "Export noisy.mp4 to 0.3 MB");
    for result in wait(&app, &jobs) {
        let path = result["path"].as_str().unwrap();
        assert!(path.contains("results"), "{path}");
        let bytes = std::fs::metadata(path).unwrap().len();
        // 0.3 MB less the 3 percent margin
        assert!(bytes <= 291_000, "{path} is {bytes}");
    }
}

#[test]
fn the_plan_is_shown_before_encoding() {
    let Some(app) = app() else { return };
    let fx = fixture().unwrap();
    let options = SizeOptions {
        target: Some(vellum_export::size::TargetSize {
            amount: 0.25,
            unit: vellum_core::settings::SizeUnit::Mb,
        }),
        ..SizeOptions::default()
    };
    let plan = app.tools.size_plan(&fx.file("fast.mp4"), &options).unwrap();
    assert!(!plan.already_fits);
    let video = plan.video.unwrap();
    assert_eq!(video.fps, Some(30.0));
    assert!(plan.summary.contains("at 30 fps"), "{}", plan.summary);

    let fits = app
        .tools
        .size_plan(&fx.file("photo.jpg"), &SizeOptions::default())
        .unwrap();
    assert!(fits.already_fits);
}

#[test]
fn gif_estimates_are_in_the_right_ballpark() {
    let Some(app) = app() else { return };
    let fx = fixture().unwrap();
    let options = AnimOptions {
        start: Some(0.0),
        end: Some(4.0),
        width: Some(320),
        ..AnimOptions::default()
    };
    let estimate = app
        .tools
        .anim_estimate(&fx.file("noisy.mp4"), &options)
        .unwrap();
    assert_eq!((estimate.width, estimate.height), (320, 180));
    assert_eq!(estimate.frames, 60);

    let id = open(&app, "video_to_gif", &[fx.file("noisy.mp4")]);
    set(
        &app,
        &id,
        json!({ "anim": { "start": 0.0, "end": 4.0, "width": 320 } }),
    );
    let result = &wait(&app, &app.tools.run(&id).unwrap())[0];
    let real = result["bytes"].as_u64().unwrap() as f64;
    let guess = estimate.bytes as f64;
    assert!(
        guess > real * 0.5 && guess < real * 2.0,
        "guessed {guess}, made {real}"
    );
}

#[test]
fn gif_tools_edit_and_split() {
    let Some(app) = app() else { return };
    let fx = fixture().unwrap();
    let gif = fx.file("anim.gif");
    let view = app.tools.gif_frames(&gif).unwrap();
    assert!(std::path::Path::new(&view.sheet).is_file());
    let frames = view.info.delays_ms.len();
    assert!(frames > 2);
    // a second look comes from the cache
    assert_eq!(app.tools.gif_frames(&gif).unwrap().sheet, view.sheet);

    let id = open(&app, "gif_tools", std::slice::from_ref(&gif));
    set(
        &app,
        &id,
        json!({ "mode": "edit", "frames": [{ "source": 1, "delayMs": 500 }, { "source": 0, "delayMs": 500 }] }),
    );
    let result = &wait(&app, &app.tools.run(&id).unwrap())[0];
    assert!(
        result["path"]
            .as_str()
            .unwrap()
            .ends_with("anim (edited).gif")
    );

    set(&app, &id, json!({ "mode": "split" }));
    let result = &wait(&app, &app.tools.run(&id).unwrap())[0];
    assert_eq!(result["folder"], json!(true));
    let count = std::fs::read_dir(result["path"].as_str().unwrap())
        .unwrap()
        .count();
    assert_eq!(count, frames);
}

#[test]
fn pictures_to_a_video_is_one_slideshow_job() {
    let Some(app) = app() else { return };
    let fx = fixture().unwrap();
    let id = open(
        &app,
        "convert",
        &[fx.file("photo.jpg"), fx.file("alpha.png")],
    );
    set(&app, &id, json!({ "target": "mp4", "slideSeconds": 1 }));
    let jobs = app.tools.run(&id).unwrap();
    assert_eq!(jobs.len(), 1);
    let result = &wait(&app, &jobs)[0];
    assert!(result["path"].as_str().unwrap().ends_with(".mp4"));
}

#[test]
fn a_session_without_files_says_what_to_do() {
    let Some(app) = app() else { return };
    let id = open(&app, "convert", &[]);
    let err = app.tools.run(&id).unwrap_err().to_string();
    assert!(err.starts_with("Add a file first"), "{err}");
}
