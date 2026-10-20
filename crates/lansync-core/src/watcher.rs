use anyhow::{Context, Result};
use notify_debouncer_full::{new_debouncer, notify::RecursiveMode, DebounceEventResult, Debouncer, FileIdMap};
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

use crate::paths::{is_ignored, normalize_rel};

/// Watches directory changes with debounce (1s default in runtime, shorter in tests).
pub struct FolderWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, FileIdMap>,
}

impl FolderWatcher {
    pub fn start<P: AsRef<Path>>(
        root: P,
        debounce: Duration,
        tx: mpsc::Sender<()>,
    ) -> Result<Self> {
        let root_path = root.as_ref().to_path_buf();
        let canonical_root = root_path.canonicalize().unwrap_or_else(|_| root_path.clone());
        let root_for_closure = root_path.clone();

        let mut debouncer = new_debouncer(
            debounce,
            None,
            move |res: DebounceEventResult| {
                match res {
                    Ok(events) => {
                        let mut has_relevant_change = false;
                        for event in events {
                            for path in &event.paths {
                                let rel_opt = normalize_rel(&canonical_root, path)
                                    .or_else(|| normalize_rel(&root_for_closure, path));

                                if let Some(rel) = rel_opt {
                                    if is_ignored(&rel) {
                                        continue;
                                    }
                                    has_relevant_change = true;
                                    break;
                                } else if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                                    if !is_ignored(file_name) {
                                        has_relevant_change = true;
                                        break;
                                    }
                                }
                            }
                            if has_relevant_change {
                                break;
                            }
                        }
                        if has_relevant_change {
                            let _ = tx.try_send(());
                        }
                    }
                    Err(errs) => {
                        for e in errs {
                            warn!("File watcher error: {:?}", e);
                        }
                    }
                }
            },
        ).context("Failed to initialize file debouncer")?;

        debouncer
            .watch(&root_path, RecursiveMode::Recursive)
            .with_context(|| format!("Failed to watch directory: {:?}", root_path))?;

        Ok(Self {
            _debouncer: debouncer,
        })
    }
}

