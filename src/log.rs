//! Logger with `[timestamp] [INFO]`/`[WARN]`/`[ERROR]`/`[DEBUG]` prefixes.
//!
//! The timestamp lets long-running `watch` log files (each watcher appends
//! to its own file — see `examples/multi-project-watch/`) be correlated
//! across projects and read for recency without relying on file mtime.
//!
//! Debug output is gated behind the `--debug` flag.

use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables or disables debug output. Called once at startup from parsed CLI flags.
pub fn set_debug(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .unwrap_or_default()
}

pub fn info(message: &str) {
    println!("[{}] [INFO] {message}", timestamp());
}

pub fn warn(message: &str) {
    println!("[{}] [WARN] {message}", timestamp());
}

pub fn error(message: &str) {
    eprintln!("[{}] [ERROR] {message}", timestamp());
}

pub fn debug(message: &str) {
    if DEBUG_ENABLED.load(Ordering::Relaxed) {
        println!("[{}] [DEBUG] {message}", timestamp());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_disabled_by_default() {
        assert!(!DEBUG_ENABLED.load(Ordering::Relaxed));
    }
}
