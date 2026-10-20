//! Decides what to sync from peer. Pure function, no I/O.
use crate::paths::case_key;
use crate::protocol::{Entry, Hash};
use crate::version_vector::VectorRelation;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Download `remote_path` to `local_path`. `expected_local` is the hash of the local version
    /// at plan time (None = absent/deleted), used to prevent overwriting if modified locally.
    Download { local_path: String, remote_path: String, hash: Hash, expected_local: Option<Hash> },
    /// Both sides match, only update base hash.
    UpdateBase { path: String, hash: Hash },
    /// Conflict where local machine loses: rename local version to conflict path and download remote to original.
    Conflict { local_path: String, conflict_path: String, remote_path: String, hash: Hash },
    /// File was renamed or moved on remote: rename local file from `from_path` to `to_path` without re-downloading data.
    Rename { from_path: String, to_path: String, hash: Hash },
    /// Move locally existing file to .lansync-trash due to remote deletion.
    Trash { path: String },
    /// Prompt user whether to delete locally existing file due to remote deletion.
    AskDelete { path: String },
}

pub struct PlanInput<'a> {
    pub local: &'a [Entry],
    pub remote: &'a [Entry],
    /// Base hash by path (for this peer pair)
    pub base: &'a HashMap<String, Hash>,
    /// Tombstones: paths deleted locally with their last known hash
    pub tombstones: &'a HashMap<String, Hash>,
    /// True if local device_id is lower than remote device_id
    pub i_win_conflicts: bool,
    pub my_name: &'a str,
    /// Timestamp formatted as "YYYY-MM-DD HHMM" for conflict naming
    pub stamp: &'a str,
    /// Remote deletion behavior
    pub delete_mode: crate::config::DeleteMode,
}

pub fn plan(input: &PlanInput) -> Vec<Action> {
    let local: HashMap<String, &Entry> = input.local.iter().map(|e| (case_key(&e.path), e)).collect();
    let base: HashMap<String, Hash> = input.base.iter().map(|(p, h)| (case_key(p), *h)).collect();
    let tombstones: HashMap<String, Hash> = input.tombstones.iter().map(|(p, h)| (case_key(p), *h)).collect();
    let remote_by_key: HashMap<String, &Entry> = input.remote.iter().map(|e| (case_key(&e.path), e)).collect();

    let mut remote: Vec<&Entry> = input.remote.iter().collect();
    remote.sort_by(|a, b| a.path.cmp(&b.path));

    let mut used_rename_sources = std::collections::HashSet::new();
    let mut actions = Vec::new();

    for r in &remote {
        // Remote deleted: do not propagate deletion
        let Some(rh) = r.hash else { continue };
        let key = case_key(&r.path);

        // 1.1 Tombstone check: if user deleted this file with exact same hash, do NOT resurrect!
        if tombstones.get(&key).is_some_and(|th| *th == rh) {
            continue;
        }

        let b = base.get(&key).copied();
        let download = |local_path: &str, expected_local| Action::Download {
            local_path: local_path.to_string(),
            remote_path: r.path.clone(),
            hash: rh,
            expected_local,
        };

        // 1.4 Rename / Move detection:
        // If file doesn't exist locally, check if local has an existing file with identical hash
        // whose remote counterpart was removed or does not exist.
        let try_find_rename_candidate = || -> Option<String> {
            for l in input.local {
                if l.hash.is_some_and(|lh| lh == rh && l.path != r.path && !used_rename_sources.contains(&l.path)) {
                    let cand_key = case_key(&l.path);
                    // Check if remote still has this old file
                    let remote_has_old = remote_by_key
                        .get(&cand_key)
                        .map(|re| re.hash.is_some())
                        .unwrap_or(false);

                    if !remote_has_old {
                        return Some(l.path.clone());
                    }
                }
            }
            None
        };

        match local.get(&key) {
            None => {
                if let Some(from_path) = try_find_rename_candidate() {
                    used_rename_sources.insert(from_path.clone());
                    actions.push(Action::Rename {
                        from_path,
                        to_path: r.path.clone(),
                        hash: rh,
                    });
                } else {
                    actions.push(download(&r.path, None));
                }
            }
            Some(l) => match l.hash {
                None => {
                    if let Some(from_path) = try_find_rename_candidate() {
                        used_rename_sources.insert(from_path.clone());
                        actions.push(Action::Rename {
                            from_path,
                            to_path: r.path.clone(),
                            hash: rh,
                        });
                    } else if b != Some(rh) {
                        actions.push(download(&l.path, None));
                    }
                }
                Some(lh) if lh == rh => {
                    if b != Some(lh) {
                        actions.push(Action::UpdateBase { path: l.path.clone(), hash: lh });
                    }
                }
                Some(lh) => {
                    let rel = l.vv.relation(&r.vv);
                    match rel {
                        VectorRelation::Older => {
                            // Remote is newer according to version vector
                            actions.push(download(&l.path, Some(lh)));
                        }
                        VectorRelation::Newer => {
                            // Local is newer according to version vector, preserve local copy
                        }
                        VectorRelation::Equal => {
                            // Fallback to base hash when version vectors are equal/empty
                            if b == Some(lh) {
                                actions.push(download(&l.path, Some(lh)));
                            } else if b == Some(rh) {
                                // Local modified, remote did not
                            } else if input.i_win_conflicts {
                                // Tiebreak: local wins
                            } else {
                                actions.push(Action::Conflict {
                                    local_path: l.path.clone(),
                                    conflict_path: conflict_name(&l.path, input.my_name, input.stamp),
                                    remote_path: r.path.clone(),
                                    hash: rh,
                                });
                            }
                        }
                        VectorRelation::Concurrent => {
                            // Concurrent modifications on both sides
                            if input.i_win_conflicts {
                                // Tiebreak: local keeps original filename
                            } else {
                                let local_cnt = l.vv.get(input.my_name);
                                let conflict_path = if local_cnt > 0 {
                                    conflict_name_with_counter(&l.path, input.my_name, local_cnt)
                                } else {
                                    conflict_name(&l.path, input.my_name, input.stamp)
                                };
                                actions.push(Action::Conflict {
                                    local_path: l.path.clone(),
                                    conflict_path,
                                    remote_path: r.path.clone(),
                                    hash: rh,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // 1.2 Remote deletion handling based on delete_mode
    if input.delete_mode != crate::config::DeleteMode::Never {
        for r in &remote {
            if r.hash.is_some() {
                continue;
            }
            let key = case_key(&r.path);
            let Some(l) = local.get(&key) else { continue };
            let Some(lh) = l.hash else { continue };

            // Skip files already handled as rename source
            if used_rename_sources.contains(&l.path) {
                continue;
            }

            let b = base.get(&key).copied();
            let rel = l.vv.relation(&r.vv);

            let is_valid_delete = match rel {
                VectorRelation::Older => true,
                VectorRelation::Equal => b == Some(lh),
                VectorRelation::Newer | VectorRelation::Concurrent => false,
            };

            if is_valid_delete {
                match input.delete_mode {
                    crate::config::DeleteMode::Never => {}
                    crate::config::DeleteMode::MirrorTrash => {
                        actions.push(Action::Trash { path: l.path.clone() });
                    }
                    crate::config::DeleteMode::Ask => {
                        actions.push(Action::AskDelete { path: l.path.clone() });
                    }
                }
            }
        }
    }

    actions
}

/// `dir/name.ext` -> `dir/name (conflict-<device> v<counter>).ext`
pub fn conflict_name_with_counter(path: &str, device_name: &str, counter: u64) -> String {
    let safe: String = device_name
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    format!("{dir}{stem} (conflict-{safe} v{counter}){ext}")
}

/// `dir/name.ext` -> `dir/name (conflict-<device> <stamp>).ext`
pub fn conflict_name(path: &str, device_name: &str, stamp: &str) -> String {
    let safe: String = device_name
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    format!("{dir}{stem} (conflict-{safe} {stamp}){ext}")
}

