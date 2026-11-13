use std::time::{SystemTime, UNIX_EPOCH};

/// wall clock time in milliseconds since the unix epoch, for timestamps shown in the ui
pub(crate) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
