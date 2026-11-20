//! the media service: previews as background jobs, cached, deduplicated, with events.

mod common;

use std::time::Duration;

use common::fixture;
use vellum_core::events::EventBus;
use vellum_core::jobs::{JobLimits, JobQueue, JobState};
use vellum_core::settings::MediaSettings;
use vellum_media::service::{AssetKind, AssetState, MediaAsset};
use vellum_media::{MediaKind, MediaService};

const WAIT: Duration = Duration::from_secs(60);

fn service(dir: &std::path::Path) -> Option<(MediaService, EventBus, JobQueue)> {
    let fx = fixture()?;
    let bus = EventBus::new();
    let jobs = JobQueue::new(JobLimits::default(), bus.clone());
    let media = MediaService::new(
        Some(fx.ffmpeg.clone()),
        &dir.join("cache"),
        &MediaSettings::default(),
        jobs.clone(),
        bus.clone(),
    );
    Some((media, bus, jobs))
}

/// waits for the event about `source` and `kind`
fn ready(
    events: &vellum_core::Subscription<MediaAsset>,
    source: &str,
    kind: AssetKind,
) -> MediaAsset {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let event = events.recv_timeout(left).expect("a media:asset event");
        if event.source == source && event.kind == kind {
            return event;
        }
    }
}

#[test]
fn previews_are_made_once_then_come_from_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let Some((media, bus, jobs)) = service(dir.path()) else {
        return;
    };
    let Some(fx) = fixture() else { return };
    let Some(video) = fx.file("h264.mp4") else {
        return;
    };
    let source = video.display().to_string();
    let events = bus.subscribe::<MediaAsset>();

    let first = media.thumbnail(video);
    assert_eq!(first.state, AssetState::Pending);
    // asking again while it runs does not start a second job
    assert_eq!(media.thumbnail(video).state, AssetState::Pending);
    assert!(jobs.list().len() <= 1);

    let done = ready(&events, &source, AssetKind::Thumbnail);
    let AssetState::Ready {
        file: Some(file),
        meta,
    } = done.state
    else {
        panic!("thumbnail failed: {done:?}");
    };
    assert!(std::path::Path::new(&file).is_file());
    // never bigger than the source, which is 320 by 240
    assert_eq!(meta.unwrap()["width"], 320);

    // now it is cached, and the background job is gone from the queue
    let again = media.thumbnail(video);
    assert!(matches!(again.state, AssetState::Ready { file: Some(ref f), .. } if *f == file));
    let deadline = std::time::Instant::now() + WAIT;
    while !jobs.list().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "background jobs stayed in the queue"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        media.cache_stats().entries >= 2,
        "thumbnail and probe are both cached"
    );
}

#[test]
fn filmstrips_and_waveforms_carry_what_the_ui_needs() {
    let dir = tempfile::tempdir().unwrap();
    let Some((media, bus, _jobs)) = service(dir.path()) else {
        return;
    };
    let Some(fx) = fixture() else { return };
    let events = bus.subscribe::<MediaAsset>();

    if let Some(video) = fx.file("h264.mp4") {
        media.filmstrip(video, 6, 72);
        let done = ready(&events, &video.display().to_string(), AssetKind::Filmstrip);
        let AssetState::Ready {
            file: Some(_),
            meta: Some(meta),
        } = done.state
        else {
            panic!("filmstrip failed: {done:?}");
        };
        assert_eq!(meta["frames"], 6);
        assert_eq!(meta["frameHeight"], 72);
    }
    if let Some(audio) = fx.file("tone.flac") {
        assert!(media.waveform_overview(audio, 100).unwrap().is_none());
        media.waveform(audio);
        let done = ready(&events, &audio.display().to_string(), AssetKind::Waveform);
        assert!(
            matches!(done.state, AssetState::Ready { file: Some(_), .. }),
            "{done:?}"
        );
        let overview = media.waveform_overview(audio, 100).unwrap().unwrap();
        assert_eq!(overview.columns, 100);
        assert_eq!(overview.channels, 1);
        // audio without cover art has nothing to show as a thumbnail, which is not a failure
        media.thumbnail(audio);
        let thumb = ready(&events, &audio.display().to_string(), AssetKind::Thumbnail);
        assert_eq!(
            thumb.state,
            AssetState::Ready {
                file: None,
                meta: None
            }
        );
    }
}

#[test]
fn failures_are_remembered_until_the_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let Some((media, bus, _jobs)) = service(dir.path()) else {
        return;
    };
    let events = bus.subscribe::<MediaAsset>();
    let broken = dir.path().join("broken.mp4");
    std::fs::write(&broken, b"definitely not a video").unwrap();
    let source = broken.display().to_string();

    media.thumbnail(&broken);
    let done = ready(&events, &source, AssetKind::Thumbnail);
    assert!(matches!(done.state, AssetState::Failed { .. }), "{done:?}");
    // straight away, no new job
    assert!(matches!(
        media.thumbnail(&broken).state,
        AssetState::Failed { .. }
    ));

    // a fixed file gets a fresh try
    let Some(fx) = fixture() else { return };
    let Some(good) = fx.file("photo.jpg") else {
        return;
    };
    std::fs::copy(good, &broken).unwrap();
    assert_eq!(media.thumbnail(&broken).state, AssetState::Pending);
}

#[test]
fn probing_is_cached_and_strip_metadata_runs_as_a_visible_job() {
    let dir = tempfile::tempdir().unwrap();
    let Some((media, _bus, jobs)) = service(dir.path()) else {
        return;
    };
    let Some(fx) = fixture() else { return };
    let Some(photo) = fx.file("exif.jpg") else {
        return;
    };
    let copy = dir.path().join("holiday.jpg");
    std::fs::copy(photo, &copy).unwrap();

    let first = media.probe(&copy).unwrap();
    let second = media.probe(&copy).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.kind, MediaKind::Image);
    assert!(first.has_location);

    let id = media.strip_metadata(&copy);
    let job = jobs.wait(id, WAIT).expect("the job finishes");
    assert_eq!(job.state, JobState::Completed, "{:?}", job.error);
    assert!(!job.background);
    let out = job.result.unwrap()["path"].as_str().unwrap().to_owned();
    assert!(out.ends_with("holiday (no metadata).jpg"));
    assert!(
        !media
            .probe(std::path::Path::new(&out))
            .unwrap()
            .has_location
    );

    let stats = media.clean_cache().unwrap();
    assert_eq!(stats.entries, 0);
}
