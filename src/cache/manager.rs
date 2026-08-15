//! File-hash cache with debounced disk persistence.
//!
//! The cache lives at `~/.cache/code-index/cache-<sha256(workspacePath)>.json`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use crate::log;

const DEBOUNCE_MS: u64 = 1500;

/// State shared with the background saver task (owned via `Arc` so the task
/// never outlives the data it writes).
struct SharedCache {
    cache_path: PathBuf,
    file_hashes: Mutex<HashMap<String, String>>,
}

impl SharedCache {
    /// Saves the cache to disk immediately.
    fn perform_save(&self) {
        let json = {
            let hashes = self.lock();
            match serde_json::to_string_pretty(&*hashes) {
                Ok(json) => json,
                Err(err) => {
                    log::error(&format!("Failed to serialize cache: {}", err));
                    return;
                }
            }
        };
        if let Some(dir) = self.cache_path.parent() {
            if let Err(err) = std::fs::create_dir_all(dir) {
                log::error(&format!("Failed to save cache: {}", err));
                return;
            }
        }
        if let Err(err) = std::fs::write(&self.cache_path, json) {
            log::error(&format!("Failed to save cache: {}", err));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        // A poisoned mutex means a panic while holding the lock; recover by
        // taking the inner data (hashes are at worst slightly stale).
        self.file_hashes
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
}

/// Manages the file hash cache for code indexing.
pub struct HashCacheManager {
    shared: Arc<SharedCache>,
    /// Monotonic generation counter implementing the debounce: the background
    /// saver only writes when no update happened within DEBOUNCE_MS.
    generation: Arc<AtomicU64>,
    save_task_spawned: Arc<AtomicBool>,
}

impl HashCacheManager {
    /// Creates a new manager for `workspace_path`. `cache_dir` overrides the
    /// default `~/.cache/code-index` directory (used by tests).
    pub fn new(workspace_path: &Path, cache_dir: Option<PathBuf>) -> Self {
        let dir = cache_dir.unwrap_or_else(default_cache_dir);
        let hash = hex::encode(Sha256::digest(workspace_path.to_string_lossy().as_bytes()));
        Self {
            shared: Arc::new(SharedCache {
                cache_path: dir.join(format!("cache-{hash}.json")),
                file_hashes: Mutex::new(HashMap::new()),
            }),
            generation: Arc::new(AtomicU64::new(0)),
            save_task_spawned: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Loads the cache file, starting fresh when missing or invalid.
    pub fn initialize(&self) {
        let data = match std::fs::read_to_string(&self.shared.cache_path) {
            Ok(data) => data,
            Err(_) => {
                log::debug(&format!(
                    "Cache file not found or invalid, starting fresh: {}",
                    self.shared.cache_path.display()
                ));
                return;
            }
        };
        match serde_json::from_str::<HashMap<String, String>>(&data) {
            Ok(hashes) => *self.shared.lock() = hashes,
            Err(_) => {
                log::debug(&format!(
                    "Cache file not found or invalid, starting fresh: {}",
                    self.shared.cache_path.display()
                ));
            }
        }
    }

    /// Clears the cache file (`clearCacheFile` in TS).
    pub fn clear_cache_file(&self) {
        self.shared.lock().clear();
        if self.shared.cache_path.exists() {
            if let Err(err) = std::fs::write(&self.shared.cache_path, "{}") {
                log::error(&format!("Failed to clear cache file: {}", err));
            }
        }
    }

    /// Schedules a debounced save. Outside a tokio runtime (e.g. pure sync
    /// tests) saves are deferred until `flush` is called instead — callers
    /// must flush on exit or the final debounce window is lost.
    fn schedule_save(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.save_task_spawned.swap(true, Ordering::SeqCst) {
            return; // a saver task is already inside its sleep window
        }

        let shared = Arc::clone(&self.shared);
        let generation_counter = Arc::clone(&self.generation);
        let spawned = Arc::clone(&self.save_task_spawned);
        handle.spawn(async move {
            // Re-read the generation each iteration: comparing against the
            // spawn-time value spins forever once any second update lands
            // in-window (and latches `spawned` = true) — review bug #1.
            let mut last_seen = generation_counter.load(Ordering::SeqCst);
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(DEBOUNCE_MS)).await;
                let now = generation_counter.load(Ordering::SeqCst);
                if now == last_seen {
                    shared.perform_save();
                    spawned.store(false, Ordering::SeqCst);
                    break;
                }
                last_seen = now;
            }
        });
    }

    pub fn cache_path(&self) -> &Path {
        &self.shared.cache_path
    }
}

fn default_cache_dir() -> PathBuf {
    std::env::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("code-index")
}

#[async_trait::async_trait]
impl crate::traits::CacheManager for HashCacheManager {
    fn get_hash(&self, file_path: &str) -> Option<String> {
        self.shared.lock().get(file_path).cloned()
    }

    fn update_hash(&self, file_path: &str, hash: String) {
        self.shared.lock().insert(file_path.to_string(), hash);
        self.schedule_save();
    }

    fn delete_hash(&self, file_path: &str) {
        self.shared.lock().remove(file_path);
        self.schedule_save();
    }

    async fn flush(&self) -> anyhow::Result<()> {
        self.shared.perform_save();
        Ok(())
    }

    fn get_all_hashes(&self) -> HashMap<String, String> {
        self.shared.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::CacheManager;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_cache_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("code-index-cache-test-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manager(tag: &str) -> HashCacheManager {
        HashCacheManager::new(Path::new("/tmp/some-workspace"), Some(temp_cache_dir(tag)))
    }

    #[test]
    fn starts_empty_when_no_cache_file() {
        let cache = manager("empty");
        cache.initialize();
        assert!(cache.get_all_hashes().is_empty());
    }

    #[test]
    fn loads_existing_hashes() {
        let dir = temp_cache_dir("load");
        let cache = HashCacheManager::new(Path::new("/tmp/some-workspace"), Some(dir));
        std::fs::write(cache.cache_path(), r#"{"src/a.ts": "hash-a"}"#).unwrap();
        cache.initialize();
        assert_eq!(cache.get_hash("src/a.ts").as_deref(), Some("hash-a"));
    }

    #[test]
    fn handles_corrupted_cache_file() {
        let dir = temp_cache_dir("corrupt");
        let cache = HashCacheManager::new(Path::new("/tmp/some-workspace"), Some(dir));
        std::fs::write(cache.cache_path(), "{ not json").unwrap();
        cache.initialize();
        assert!(cache.get_all_hashes().is_empty());
    }

    #[test]
    fn caches_by_workspace_path() {
        let dir = temp_cache_dir("paths");
        let a = HashCacheManager::new(Path::new("/tmp/ws-a"), Some(dir.clone()));
        let b = HashCacheManager::new(Path::new("/tmp/ws-b"), Some(dir));
        assert_ne!(a.cache_path(), b.cache_path());
        assert!(a
            .cache_path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("cache-"));
    }

    #[test]
    fn hash_crud() {
        let cache = manager("crud");
        assert_eq!(cache.get_hash("x"), None);

        cache.update_hash("a.ts", "h1".into());
        assert_eq!(cache.get_hash("a.ts").as_deref(), Some("h1"));

        cache.update_hash("a.ts", "h2".into());
        assert_eq!(cache.get_hash("a.ts").as_deref(), Some("h2"));

        cache.delete_hash("a.ts");
        assert_eq!(cache.get_hash("a.ts"), None);

        // Deleting a non-existent hash must not panic
        cache.delete_hash("never-present.ts");
    }

    #[test]
    fn get_all_hashes_returns_a_copy() {
        let cache = manager("copy");
        cache.update_hash("a.ts", "h1".into());
        let mut all = cache.get_all_hashes();
        all.insert("injected.ts".into(), "x".into());
        assert_eq!(cache.get_hash("injected.ts"), None);
    }

    #[tokio::test]
    async fn flush_persists_to_disk() {
        let cache = manager("flush");
        cache.update_hash("a.ts", "h1".into());
        cache.flush().await.unwrap();
        let written = std::fs::read_to_string(cache.cache_path()).unwrap();
        assert!(written.contains("\"a.ts\": \"h1\""));
    }

    #[tokio::test]
    async fn debounced_save_eventually_persists() {
        let cache = manager("debounce");
        cache.update_hash("b.ts", "hb".into());
        tokio::time::sleep(std::time::Duration::from_millis(DEBOUNCE_MS + 500)).await;
        assert!(cache.cache_path().exists());
    }

    /// Regression: two updates inside one debounce window must still save —
    /// previously the task compared against its spawn-time generation and
    /// spun forever once any second update landed (review bug #1).
    #[tokio::test]
    async fn repeated_in_window_updates_still_save() {
        let cache = manager("two-updates");
        cache.update_hash("a.ts", "h1".into());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        cache.update_hash("b.ts", "h2".into());

        // window from the last update, plus margin
        tokio::time::sleep(std::time::Duration::from_millis(2 * DEBOUNCE_MS + 500)).await;
        assert!(cache.cache_path().exists());

        // and subsequent updates continue to debounce-save normally
        cache.update_hash("c.ts", "h3".into());
        tokio::time::sleep(std::time::Duration::from_millis(DEBOUNCE_MS + 500)).await;
        let written = std::fs::read_to_string(cache.cache_path()).unwrap();
        assert!(written.contains("h3"));
    }

    #[test]
    fn clear_cache_file_empties_and_resets_file() {
        let dir = temp_cache_dir("clear");
        let cache = HashCacheManager::new(Path::new("/tmp/some-workspace"), Some(dir));
        cache.update_hash("a.ts", "h1".into());
        cache.shared.perform_save();

        cache.clear_cache_file();
        assert!(cache.get_all_hashes().is_empty());
        assert_eq!(std::fs::read_to_string(cache.cache_path()).unwrap(), "{}");
    }

    #[test]
    fn clear_cache_file_without_existing_file_is_noop() {
        let cache = manager("clear-noop");
        cache.clear_cache_file(); // must not panic
    }
}
