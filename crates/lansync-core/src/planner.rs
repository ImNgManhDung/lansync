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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version_vector::VersionVector;

    fn h(n: u8) -> Hash { [n; 32] }
    fn e(path: &str, hash: Option<u8>) -> Entry {
        Entry::simple(path, 1, 1, hash.map(h))
    }
    fn run(local: &[Entry], remote: &[Entry], base: &[(&str, u8)], win: bool) -> Vec<Action> {
        let base: HashMap<String, Hash> = base.iter().map(|(p, n)| (p.to_string(), h(*n))).collect();
        let tombstones = HashMap::new();
        plan(&PlanInput {
            local,
            remote,
            base: &base,
            tombstones: &tombstones,
            i_win_conflicts: win,
            my_name: "Mac",
            stamp: "2026-10-13 1705",
            delete_mode: crate::config::DeleteMode::Never,
        })
    }

    fn run_with_tombstones(local: &[Entry], remote: &[Entry], base: &[(&str, u8)], tombstones: &[(&str, u8)], win: bool) -> Vec<Action> {
        let base: HashMap<String, Hash> = base.iter().map(|(p, n)| (p.to_string(), h(*n))).collect();
        let tombstones: HashMap<String, Hash> = tombstones.iter().map(|(p, n)| (p.to_string(), h(*n))).collect();
        plan(&PlanInput {
            local,
            remote,
            base: &base,
            tombstones: &tombstones,
            i_win_conflicts: win,
            my_name: "Mac",
            stamp: "2026-10-13 1705",
            delete_mode: crate::config::DeleteMode::Never,
        })
    }

    fn run_with_mode(
        local: &[Entry],
        remote: &[Entry],
        base: &[(&str, u8)],
        mode: crate::config::DeleteMode,
    ) -> Vec<Action> {
        let base: HashMap<String, Hash> = base.iter().map(|(p, n)| (p.to_string(), h(*n))).collect();
        let tombstones = HashMap::new();
        plan(&PlanInput {
            local,
            remote,
            base: &base,
            tombstones: &tombstones,
            i_win_conflicts: false,
            my_name: "Mac",
            stamp: "2026-10-13 1705",
            delete_mode: mode,
        })
    }

    #[test]
    fn tombstone_prevents_deleted_file_resurrection_unless_modified_remotely() {
        // Mac deleted a.pdf which had hash 1. Remote still has hash 1.
        // Planner MUST NOT download it!
        let actions = run_with_tombstones(&[], &[e("a.pdf", Some(1))], &[], &[("a.pdf", 1)], false);
        assert!(actions.is_empty(), "Tombstone matching remote hash must not download!");

        // If remote modified a.pdf to hash 2, it is a new version and SHOULD download!
        let actions_modified = run_with_tombstones(&[], &[e("a.pdf", Some(2))], &[], &[("a.pdf", 1)], false);
        assert_eq!(actions_modified, vec![Action::Download {
            local_path: "a.pdf".into(), remote_path: "a.pdf".into(), hash: h(2), expected_local: None }]);
    }

    #[test]
    fn remote_deleted_does_nothing() {
        assert!(run(&[e("a", Some(1))], &[e("a", None)], &[("a", 1)], false).is_empty());
    }

    #[test]
    fn missing_locally_downloads() {
        assert_eq!(run(&[], &[e("a", Some(1))], &[], false), vec![Action::Download {
            local_path: "a".into(), remote_path: "a".into(), hash: h(1), expected_local: None }]);
    }

    #[test]
    fn locally_deleted_and_remote_unchanged_is_respected() {
        assert!(run(&[e("a", None)], &[e("a", Some(1))], &[("a", 1)], false).is_empty());
    }

    #[test]
    fn locally_deleted_but_remote_changed_downloads() {
        assert_eq!(run(&[e("a", None)], &[e("a", Some(2))], &[("a", 1)], false), vec![Action::Download {
            local_path: "a".into(), remote_path: "a".into(), hash: h(2), expected_local: None }]);
    }

    #[test]
    fn same_hash_updates_base_only_when_needed() {
        assert_eq!(run(&[e("a", Some(1))], &[e("a", Some(1))], &[], false),
                   vec![Action::UpdateBase { path: "a".into(), hash: h(1) }]);
        assert!(run(&[e("a", Some(1))], &[e("a", Some(1))], &[("a", 1)], false).is_empty());
    }

    #[test]
    fn only_remote_changed_downloads() {
        assert_eq!(run(&[e("a", Some(1))], &[e("a", Some(2))], &[("a", 1)], false), vec![Action::Download {
            local_path: "a".into(), remote_path: "a".into(), hash: h(2), expected_local: Some(h(1)) }]);
    }

    #[test]
    fn only_local_changed_does_nothing() {
        assert!(run(&[e("a", Some(2))], &[e("a", Some(1))], &[("a", 1)], false).is_empty());
    }

    #[test]
    fn both_changed_loser_creates_conflict_winner_waits() {
        let local = [e("docs/report.docx", Some(2))];
        let remote = [e("docs/report.docx", Some(3))];
        assert_eq!(run(&local, &remote, &[("docs/report.docx", 1)], false), vec![Action::Conflict {
            local_path: "docs/report.docx".into(),
            conflict_path: "docs/report (conflict-Mac 2026-10-13 1705).docx".into(),
            remote_path: "docs/report.docx".into(),
            hash: h(3),
        }]);
        assert!(run(&local, &remote, &[("docs/report.docx", 1)], true).is_empty());
    }

    #[test]
    fn no_base_and_different_is_conflict() {
        let a = run(&[e("a.txt", Some(2))], &[e("a.txt", Some(3))], &[], false);
        assert!(matches!(&a[..], [Action::Conflict { .. }]));
    }

    #[test]
    fn matching_is_case_insensitive_and_keeps_local_name() {
        assert_eq!(run(&[e("Docs/A.txt", Some(1))], &[e("docs/a.txt", Some(2))], &[("DOCS/a.TXT", 1)], false),
            vec![Action::Download { local_path: "Docs/A.txt".into(), remote_path: "docs/a.txt".into(),
                                    hash: h(2), expected_local: Some(h(1)) }]);
    }

    #[test]
    fn conflict_names() {
        let s = "2026-10-13 1705";
        assert_eq!(conflict_name("a/report.docx", "Mac", s), "a/report (conflict-Mac 2026-10-13 1705).docx");
        assert_eq!(conflict_name("Makefile", "Mac", s), "Makefile (conflict-Mac 2026-10-13 1705)");
        assert_eq!(conflict_name(".env", "Mac", s), ".env (conflict-Mac 2026-10-13 1705)");
        assert_eq!(conflict_name("x.tar.gz", "PC:1/2", s), "x.tar (conflict-PC_1_2 2026-10-13 1705).gz");
    }

    #[test]
    fn rename_detection_avoids_re_download() {
        // Local has old_name.iso (hash 77). Remote deleted old_name.iso and created new_name.iso (hash 77).
        let local = [e("old_name.iso", Some(77))];
        let remote = [e("new_name.iso", Some(77))];
        let actions = run(&local, &remote, &[], false);

        assert_eq!(actions, vec![Action::Rename {
            from_path: "old_name.iso".into(),
            to_path: "new_name.iso".into(),
            hash: h(77),
        }]);
    }

    #[test]
    fn version_vector_resolves_newer_remote_despite_clock_skew() {
        // Remote mtime is 1,000,000 ns (in the past), Local mtime is 999,999,999,000 ns (in future).
        // But Remote vv is { "peer_b": 2 }, Local vv is { "peer_b": 1 }.
        // Version vector dictates Remote is Newer -> planner downloads without conflict!
        let mut vv_local = VersionVector::new();
        vv_local.set("peer_b", 1);
        let mut local_entry = Entry::simple("shared.doc", 100, 999_999_999_000, Some(h(1)));
        local_entry.vv = vv_local;

        let mut vv_remote = VersionVector::new();
        vv_remote.set("peer_b", 2);
        let mut remote_entry = Entry::simple("shared.doc", 150, 1_000_000, Some(h(2)));
        remote_entry.vv = vv_remote;

        let actions = run(&[local_entry], &[remote_entry], &[], false);
        assert_eq!(actions, vec![Action::Download {
            local_path: "shared.doc".into(),
            remote_path: "shared.doc".into(),
            hash: h(2),
            expected_local: Some(h(1)),
        }]);
    }

    #[test]
    fn version_vector_resolves_newer_local_despite_clock_skew() {
        // Remote has future mtime, but local vv dominates remote vv.
        // Local copy is preserved; planner takes no download action.
        let mut vv_local = VersionVector::new();
        vv_local.set("dev_a", 3);
        let mut local_entry = Entry::simple("notes.txt", 100, 1_000, Some(h(1)));
        local_entry.vv = vv_local;

        let mut vv_remote = VersionVector::new();
        vv_remote.set("dev_a", 2);
        let mut remote_entry = Entry::simple("notes.txt", 120, 999_999_999_000, Some(h(2)));
        remote_entry.vv = vv_remote;

        let actions = run(&[local_entry], &[remote_entry], &[], false);
        assert!(actions.is_empty(), "Local dominates, no download needed");
    }

    #[test]
    fn version_vector_concurrent_produces_counter_conflict_file() {
        // Local has { "dev_a": 2 }, Remote has { "dev_b": 1 }.
        // Neither dominates: Concurrent modification!
        let mut vv_local = VersionVector::new();
        vv_local.set("dev_a", 2);
        let mut local_entry = Entry::simple("code.rs", 50, 100, Some(h(1)));
        local_entry.vv = vv_local;

        let mut vv_remote = VersionVector::new();
        vv_remote.set("dev_b", 1);
        let mut remote_entry = Entry::simple("code.rs", 60, 200, Some(h(2)));
        remote_entry.vv = vv_remote;

        let actions = run(&[local_entry], &[remote_entry], &[], false);
        assert_eq!(actions, vec![Action::Conflict {
            local_path: "code.rs".into(),
            conflict_path: "code (conflict-Mac 2026-10-13 1705).rs".into(),
            remote_path: "code.rs".into(),
            hash: h(2),
        }]);

        // When device name matches local device and counter > 0:
        let conflict_name = conflict_name_with_counter("code.rs", "MacBook", 3);
        assert_eq!(conflict_name, "code (conflict-MacBook v3).rs");
    }

    #[test]
    fn test_delete_mode_never_ignores_remote_delete() {
        let actions = run_with_mode(
            &[e("doc.pdf", Some(1))],
            &[e("doc.pdf", None)],
            &[("doc.pdf", 1)],
            crate::config::DeleteMode::Never,
        );
        assert!(actions.is_empty(), "DeleteMode::Never must ignore remote deletion");
    }

    #[test]
    fn test_delete_mode_mirror_trash_trashes_unmodified_local_file() {
        let actions = run_with_mode(
            &[e("doc.pdf", Some(1))],
            &[e("doc.pdf", None)],
            &[("doc.pdf", 1)],
            crate::config::DeleteMode::MirrorTrash,
        );
        assert_eq!(actions, vec![Action::Trash { path: "doc.pdf".into() }]);
    }

    #[test]
    fn test_delete_mode_ask_requests_confirmation() {
        let actions = run_with_mode(
            &[e("doc.pdf", Some(1))],
            &[e("doc.pdf", None)],
            &[("doc.pdf", 1)],
            crate::config::DeleteMode::Ask,
        );
        assert_eq!(actions, vec![Action::AskDelete { path: "doc.pdf".into() }]);
    }

    #[test]
    fn test_delete_mode_mirror_trash_with_newer_remote_version_vector() {
        let mut vv_local = VersionVector::new();
        vv_local.set("peer_b", 1);
        let mut local_entry = Entry::simple("photo.jpg", 100, 100, Some(h(1)));
        local_entry.vv = vv_local;

        let mut vv_remote = VersionVector::new();
        vv_remote.set("peer_b", 2);
        let mut remote_entry = Entry::simple("photo.jpg", 0, 0, None);
        remote_entry.vv = vv_remote;

        let actions = run_with_mode(
            &[local_entry],
            &[remote_entry],
            &[],
            crate::config::DeleteMode::MirrorTrash,
        );
        assert_eq!(actions, vec![Action::Trash { path: "photo.jpg".into() }]);
    }

    #[test]
    fn test_delete_mode_protects_locally_modified_file() {
        // Local changed hash to 2, base was 1, remote deleted
        let actions = run_with_mode(
            &[e("doc.pdf", Some(2))],
            &[e("doc.pdf", None)],
            &[("doc.pdf", 1)],
            crate::config::DeleteMode::MirrorTrash,
        );
        assert!(actions.is_empty(), "Locally modified file must not be trashed");
    }

    #[test]
    fn test_delete_mode_does_not_trash_file_handled_as_rename() {
        // Local has old.txt (hash 10). Remote deleted old.txt and created new.txt (hash 10).
        let actions = run_with_mode(
            &[e("old.txt", Some(10))],
            &[e("new.txt", Some(10)), e("old.txt", None)],
            &[],
            crate::config::DeleteMode::MirrorTrash,
        );
        assert_eq!(actions, vec![Action::Rename {
            from_path: "old.txt".into(),
            to_path: "new.txt".into(),
            hash: h(10),
        }]);
    }
}
