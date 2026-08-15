//! Logger with `[INFO]`/`[WARN]`/`[ERROR]`/`[DEBUG]` prefixes.
//!
//! Port of `src/utils/logger.ts` — debug output is gated behind the `--debug` flag.

// Some helpers are unused until command handlers wire in (Phase 1+).
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables or disables debug output. Called once at startup from parsed CLI flags.
pub fn set_debug(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn info(message: &str) {
    println!("[INFO] {message}");
}

pub fn warn(message: &str) {
    println!("[WARN] {message}");
}

pub fn error(message: &str) {
    eprintln!("[ERROR] {message}");
}

pub fn debug(message: &str) {
    if DEBUG_ENABLED.load(Ordering::Relaxed) {
        println!("[DEBUG] {message}");
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
