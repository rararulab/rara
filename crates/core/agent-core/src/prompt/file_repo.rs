use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::types::{NotFoundSnafu, PromptEntry, PromptError, PromptSpec, WatcherSnafu};
use super::PromptRepo;
use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::RwLock;

/// File-system backed prompt repository with in-memory cache and fs-notify watcher.
pub struct FilePromptRepo {
    prompt_dir: PathBuf,
    registry: HashMap<String, PromptSpec>,
    cache: Arc<RwLock<HashMap<String, PromptEntry>>>,
    _watcher_handle: Option<tokio::task::JoinHandle<()>>,
}

impl FilePromptRepo {
    /// Create a new `FilePromptRepo`.
    ///
    /// 1. Creates `prompt_dir` if it doesn't exist.
    /// 2. For each spec, reads the on-disk file (or writes the default if missing/empty).
    /// 3. Loads all entries into the in-memory cache.
    /// 4. Starts a background `notify::RecommendedWatcher` to keep the cache fresh.
    pub async fn new(prompt_dir: PathBuf, specs: Vec<PromptSpec>) -> Result<Self, PromptError> {
        tokio::fs::create_dir_all(&prompt_dir)
            .await
            .map_err(|e| PromptError::Io { source: e })?;

        let mut registry = HashMap::with_capacity(specs.len());
        let mut initial_cache = HashMap::with_capacity(specs.len());

        for spec in specs {
            let file_path = prompt_dir.join(spec.name);

            // Ensure parent directories exist.
            if let Some(parent) = file_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| PromptError::Io { source: e })?;
            }

            let content = match tokio::fs::read_to_string(&file_path).await {
                Ok(c) if !c.trim().is_empty() => c,
                _ => {
                    // File missing or empty — write default.
                    tokio::fs::write(&file_path, spec.default_content)
                        .await
                        .map_err(|e| PromptError::Io { source: e })?;
                    spec.default_content.to_owned()
                }
            };

            initial_cache.insert(
                spec.name.to_owned(),
                PromptEntry {
                    name: spec.name.to_owned(),
                    description: spec.description.to_owned(),
                    content,
                },
            );
            registry.insert(spec.name.to_owned(), spec);
        }

        let cache = Arc::new(RwLock::new(initial_cache));
        let watcher_handle = Self::start_watcher(&prompt_dir, &registry, Arc::clone(&cache))?;

        Ok(Self {
            prompt_dir,
            registry,
            cache,
            _watcher_handle: Some(watcher_handle),
        })
    }

    /// Start a background watcher that monitors `prompt_dir` for changes
    /// and updates the cache accordingly.
    ///
    /// Uses a two-channel pattern to avoid blocking the async runtime:
    /// 1. `notify::RecommendedWatcher` sends to a `std::sync::mpsc` channel
    /// 2. A `spawn_blocking` task bridges events to a `tokio::sync::mpsc` channel
    /// 3. An async task reads from the tokio channel and updates the cache
    fn start_watcher(
        prompt_dir: &Path,
        registry: &HashMap<String, PromptSpec>,
        cache: Arc<RwLock<HashMap<String, PromptEntry>>>,
    ) -> Result<tokio::task::JoinHandle<()>, PromptError> {
        let (sync_tx, sync_rx) = std::sync::mpsc::channel();

        let mut watcher = notify::recommended_watcher(sync_tx)
            .map_err(|e| WatcherSnafu { message: e.to_string() }.build())?;

        watcher
            .watch(prompt_dir, RecursiveMode::Recursive)
            .map_err(|e| WatcherSnafu { message: e.to_string() }.build())?;

        // Bridge: spawn_blocking drains the std channel into a tokio channel.
        let (async_tx, mut async_rx) = tokio::sync::mpsc::channel::<notify::Event>(64);
        tokio::task::spawn_blocking(move || {
            // Keep the watcher alive for the lifetime of this blocking task.
            let _watcher = watcher;

            while let Ok(event_result) = sync_rx.recv() {
                match event_result {
                    Ok(event) => {
                        if async_tx.blocking_send(event).is_err() {
                            // Receiver dropped — stop.
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "prompt fs watcher error");
                    }
                }
            }
            tracing::debug!("prompt fs watcher channel closed, stopping bridge");
        });

        // Build a map from file path -> prompt name for fast lookup.
        let mut path_to_name: HashMap<PathBuf, String> = HashMap::new();
        for (name, _spec) in registry {
            let file_path = prompt_dir.join(name);
            path_to_name.insert(file_path, name.clone());
        }

        let prompt_dir_owned = prompt_dir.to_owned();

        // Async task: reads events from the tokio channel and updates cache.
        let handle = tokio::task::spawn(async move {
            while let Some(event) = async_rx.recv().await {
                if !matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_)
                ) {
                    continue;
                }

                for path in &event.paths {
                    let name = path_to_name.get(path).cloned().or_else(|| {
                        // Try matching via relative path.
                        path.strip_prefix(&prompt_dir_owned)
                            .ok()
                            .and_then(|rel| rel.to_str())
                            .and_then(|rel| {
                                let normalized = rel.replace('\\', "/");
                                if path_to_name.values().any(|n| n == &normalized) {
                                    Some(normalized)
                                } else {
                                    None
                                }
                            })
                    });

                    let Some(name) = name else { continue };

                    match tokio::fs::read_to_string(path).await {
                        Ok(content) if !content.trim().is_empty() => {
                            let mut guard = cache.write().await;
                            if let Some(entry) = guard.get_mut(&name) {
                                entry.content = content;
                                tracing::debug!(prompt = %name, "prompt cache refreshed via watcher");
                            }
                        }
                        _ => {
                            tracing::trace!(prompt = %name, "watcher: file empty or unreadable, skipping");
                        }
                    }
                }
            }
        });

        Ok(handle)
    }
}

#[async_trait::async_trait]
impl PromptRepo for FilePromptRepo {
    async fn get(&self, name: &str) -> Option<PromptEntry> {
        self.cache.read().await.get(name).cloned()
    }

    async fn list(&self) -> Vec<PromptEntry> {
        self.cache.read().await.values().cloned().collect()
    }

    async fn update(&self, name: &str, content: &str) -> Result<PromptEntry, PromptError> {
        let spec = self
            .registry
            .get(name)
            .ok_or_else(|| NotFoundSnafu { name }.build())?;

        let file_path = self.prompt_dir.join(name);

        // Ensure parent directory exists.
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| PromptError::Io { source: e })?;
        }

        tokio::fs::write(&file_path, content)
            .await
            .map_err(|e| PromptError::Io { source: e })?;

        let entry = PromptEntry {
            name: name.to_owned(),
            description: spec.description.to_owned(),
            content: content.to_owned(),
        };

        // Update cache immediately (don't wait for watcher).
        self.cache
            .write()
            .await
            .insert(name.to_owned(), entry.clone());

        tracing::debug!(prompt = %name, "prompt updated");
        Ok(entry)
    }

    async fn reset(&self, name: &str) -> Result<PromptEntry, PromptError> {
        let spec = self
            .registry
            .get(name)
            .ok_or_else(|| NotFoundSnafu { name }.build())?;

        let file_path = self.prompt_dir.join(name);

        tokio::fs::write(&file_path, spec.default_content)
            .await
            .map_err(|e| PromptError::Io { source: e })?;

        let entry = PromptEntry {
            name: name.to_owned(),
            description: spec.description.to_owned(),
            content: spec.default_content.to_owned(),
        };

        self.cache
            .write()
            .await
            .insert(name.to_owned(), entry.clone());

        tracing::debug!(prompt = %name, "prompt reset to default");
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_specs() -> Vec<PromptSpec> {
        vec![
            PromptSpec {
                name: "test/hello.md",
                description: "Test prompt",
                default_content: "Hello, world!",
            },
            PromptSpec {
                name: "test/nested/deep.md",
                description: "Nested prompt",
                default_content: "Deep content",
            },
        ]
    }

    #[tokio::test]
    async fn new_creates_files_and_loads_cache() {
        let dir = tempfile::tempdir().unwrap();
        let repo = FilePromptRepo::new(dir.path().to_owned(), test_specs())
            .await
            .unwrap();

        // Files should exist.
        assert!(dir.path().join("test/hello.md").exists());
        assert!(dir.path().join("test/nested/deep.md").exists());

        // Cache should be populated.
        let entries = repo.list().await;
        assert_eq!(entries.len(), 2);

        let hello = repo.get("test/hello.md").await.unwrap();
        assert_eq!(hello.content, "Hello, world!");
        assert_eq!(hello.description, "Test prompt");
    }

    #[tokio::test]
    async fn new_reads_existing_file_instead_of_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test/hello.md");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "Custom content").unwrap();

        let repo = FilePromptRepo::new(dir.path().to_owned(), test_specs())
            .await
            .unwrap();

        let entry = repo.get("test/hello.md").await.unwrap();
        assert_eq!(entry.content, "Custom content");
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let repo = FilePromptRepo::new(dir.path().to_owned(), test_specs())
            .await
            .unwrap();

        assert!(repo.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn update_writes_file_and_updates_cache() {
        let dir = tempfile::tempdir().unwrap();
        let repo = FilePromptRepo::new(dir.path().to_owned(), test_specs())
            .await
            .unwrap();

        let entry = repo.update("test/hello.md", "Updated!").await.unwrap();
        assert_eq!(entry.content, "Updated!");

        // Cache should reflect the update.
        let cached = repo.get("test/hello.md").await.unwrap();
        assert_eq!(cached.content, "Updated!");

        // File on disk should reflect the update.
        let on_disk = tokio::fs::read_to_string(dir.path().join("test/hello.md"))
            .await
            .unwrap();
        assert_eq!(on_disk, "Updated!");
    }

    #[tokio::test]
    async fn update_returns_not_found_for_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let repo = FilePromptRepo::new(dir.path().to_owned(), test_specs())
            .await
            .unwrap();

        let err = repo.update("nonexistent", "content").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn reset_restores_default() {
        let dir = tempfile::tempdir().unwrap();
        let repo = FilePromptRepo::new(dir.path().to_owned(), test_specs())
            .await
            .unwrap();

        // First update, then reset.
        repo.update("test/hello.md", "Modified").await.unwrap();
        let entry = repo.reset("test/hello.md").await.unwrap();
        assert_eq!(entry.content, "Hello, world!");

        let cached = repo.get("test/hello.md").await.unwrap();
        assert_eq!(cached.content, "Hello, world!");

        let on_disk = tokio::fs::read_to_string(dir.path().join("test/hello.md"))
            .await
            .unwrap();
        assert_eq!(on_disk, "Hello, world!");
    }

    #[tokio::test]
    async fn reset_returns_not_found_for_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let repo = FilePromptRepo::new(dir.path().to_owned(), test_specs())
            .await
            .unwrap();

        let err = repo.reset("nonexistent").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}
