//! Indexing state machine with progress reporting.
//!
//! Mutex-protected status plus subscriber callbacks (invoked outside the
//! lock so subscribers can call back in).

use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexingState {
    Standby,
    Indexing,
    Indexed,
    Error,
    Stopping,
}

impl std::fmt::Display for IndexingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Standby => "Standby",
            Self::Indexing => "Indexing",
            Self::Indexed => "Indexed",
            Self::Error => "Error",
            Self::Stopping => "Stopping",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexingStatus {
    pub system_status: IndexingState,
    pub message: String,
    pub processed_items: usize,
    pub total_items: usize,
    pub current_item_unit: String,
}

pub type StatusSubscriber = Box<dyn Fn(&IndexingStatus) + Send>;

/// Manages the state of the indexing process.
#[derive(Default)]
pub struct StateManager {
    status: Mutex<IndexingStatus>,
    subscribers: Mutex<Vec<StatusSubscriber>>,
}

impl StateManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> IndexingState {
        self.lock().system_status
    }

    pub fn current_status(&self) -> IndexingStatus {
        self.lock().clone()
    }

    pub fn on_progress_update(&self, subscriber: StatusSubscriber) {
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(subscriber);
    }

    /// Sets the system state.
    pub fn set_system_state(&self, new_state: IndexingState, message: Option<&str>) {
        let snapshot = {
            let mut status = self.lock();
            let state_changed =
                new_state != status.system_status || message.is_some_and(|m| m != status.message);

            if !state_changed {
                return;
            }

            status.system_status = new_state;
            if let Some(m) = message {
                status.message = m.to_string();
            }

            if new_state != IndexingState::Indexing {
                status.processed_items = 0;
                status.total_items = 0;
                status.current_item_unit = "blocks".to_string();
                match (new_state, message) {
                    (IndexingState::Standby, None) => status.message = "Ready.".to_string(),
                    (IndexingState::Indexed, None) => {
                        status.message = "Index up-to-date.".to_string()
                    }
                    (IndexingState::Error, None) => {
                        status.message = "An error occurred.".to_string()
                    }
                    _ => {}
                }
            }

            status.clone()
        };
        self.notify(&snapshot);
    }

    /// Reports progress on block indexing.
    pub fn report_block_indexing_progress(&self, processed_items: usize, total_items: usize) {
        let snapshot = {
            let mut status = self.lock();
            if status.system_status == IndexingState::Stopping {
                return;
            }
            let progress_changed =
                processed_items != status.processed_items || total_items != status.total_items;
            if !progress_changed && status.system_status == IndexingState::Indexing {
                return;
            }

            status.processed_items = processed_items;
            status.total_items = total_items;
            status.current_item_unit = "blocks".to_string();

            let old_status = status.system_status;
            let old_message = status.message.clone();

            status.system_status = IndexingState::Indexing;
            status.message = format!(
                "Indexed {} / {} {} found",
                status.processed_items, status.total_items, status.current_item_unit
            );

            if old_status == status.system_status
                && old_message == status.message
                && !progress_changed
            {
                return;
            }
            status.clone()
        };
        self.notify(&snapshot);
    }

    /// Reports progress on file queue processing.
    pub fn report_file_queue_progress(
        &self,
        processed_files: usize,
        total_files: usize,
        current_file_basename: Option<&str>,
    ) {
        let snapshot = {
            let mut status = self.lock();
            if status.system_status == IndexingState::Stopping {
                return;
            }
            let progress_changed =
                processed_files != status.processed_items || total_files != status.total_items;
            if !progress_changed && status.system_status == IndexingState::Indexing {
                return;
            }

            status.processed_items = processed_files;
            status.total_items = total_files;
            status.current_item_unit = "files".to_string();
            status.system_status = IndexingState::Indexing;

            status.message = if total_files > 0 && processed_files < total_files {
                format!(
                    "Processing {} / {} files. Current: {}",
                    processed_files,
                    total_files,
                    current_file_basename.unwrap_or("...")
                )
            } else if total_files > 0 && processed_files == total_files {
                format!("Finished processing {} files from queue.", total_files)
            } else {
                "No files to process.".to_string()
            };

            status.clone()
        };
        self.notify(&snapshot);
    }

    fn notify(&self, snapshot: &IndexingStatus) {
        for subscriber in self
            .subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            subscriber(snapshot);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, IndexingStatus> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for IndexingStatus {
    fn default() -> Self {
        Self {
            system_status: IndexingState::Standby,
            message: String::new(),
            processed_items: 0,
            total_items: 0,
            current_item_unit: "blocks".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    fn manager() -> StateManager {
        StateManager::new()
    }

    #[test]
    fn starts_in_standby() {
        let m = manager();
        assert_eq!(m.state(), IndexingState::Standby);
    }

    #[test]
    fn set_state_updates_message_and_resets_counters() {
        let m = manager();
        m.report_block_indexing_progress(5, 10);
        m.set_system_state(IndexingState::Indexed, None);
        let status = m.current_status();
        assert_eq!(status.system_status, IndexingState::Indexed);
        assert_eq!(status.message, "Index up-to-date.");
        assert_eq!(status.processed_items, 0);
        assert_eq!(status.total_items, 0);
    }

    #[test]
    fn set_state_with_same_state_and_no_message_is_noop() {
        let m = manager();
        let count = Arc::new(StdMutex::new(0usize));
        let counter = Arc::clone(&count);
        m.on_progress_update(Box::new(move |_| *counter.lock().unwrap() += 1));
        // Both calls are no-ops: state is already Standby and no message was
        // given.
        m.set_system_state(IndexingState::Standby, None);
        m.set_system_state(IndexingState::Standby, None);
        assert_eq!(*count.lock().unwrap(), 0);
    }

    #[test]
    fn default_messages_per_state() {
        let m = manager();
        m.set_system_state(IndexingState::Error, None);
        assert_eq!(m.current_status().message, "An error occurred.");
    }

    #[test]
    fn block_progress_moves_to_indexing() {
        let m = manager();
        m.report_block_indexing_progress(3, 10);
        let status = m.current_status();
        assert_eq!(status.system_status, IndexingState::Indexing);
        assert_eq!(status.message, "Indexed 3 / 10 blocks found");
    }

    #[test]
    fn block_progress_ignored_while_stopping() {
        let m = manager();
        m.set_system_state(IndexingState::Stopping, None);
        m.report_block_indexing_progress(3, 10);
        assert_eq!(m.state(), IndexingState::Stopping);
        assert_eq!(m.current_status().processed_items, 0);
    }

    #[test]
    fn file_queue_progress_messages() {
        let m = manager();
        m.report_file_queue_progress(2, 5, Some("main.rs"));
        assert_eq!(
            m.current_status().message,
            "Processing 2 / 5 files. Current: main.rs"
        );

        m.report_file_queue_progress(5, 5, None);
        assert_eq!(
            m.current_status().message,
            "Finished processing 5 files from queue."
        );

        m.set_system_state(IndexingState::Standby, None);
        m.report_file_queue_progress(0, 0, None);
        assert_eq!(m.current_status().message, "No files to process.");
    }
}
