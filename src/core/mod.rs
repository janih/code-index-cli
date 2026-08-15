//! Business logic: orchestrator, search service, service factory, state.
//!
//! Port of `src/core/*`. The Node EventEmitter becomes Mutex-protected
//! status with subscriber callbacks (see `state_manager`).

pub mod orchestrator;
pub mod search_service;
pub mod service_factory;
pub mod state_manager;

pub use orchestrator::Orchestrator;
pub use search_service::SearchService;
pub use service_factory::ServiceFactory;
pub use state_manager::{IndexingState, StateManager};
