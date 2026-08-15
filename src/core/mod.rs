//! Business logic: orchestrator, search service, service factory, state.
//!
//! Port of `src/core/*`. The EventEmitter-based state machine becomes an
//! enum observed via `tokio::sync::watch`.
