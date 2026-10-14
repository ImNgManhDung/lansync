//! Decides what to sync from peer. Pure function, no I/O.
use crate::paths::case_key;
use crate::protocol::{Entry, Hash};
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
}

pub struct PlanInput<'a> {
    pub local: &'a [Entry],
    pub remote: &'a [Entry],
    /// Base hash by path (for this peer pair)
    pub base: &'a HashMap<String, Hash>,
    /// True if local device_id is lower than remote device_id
    pub i_win_conflicts: bool,
    pub my_name: &'a str,
    /// Timestamp formatted as "YYYY-MM-DD HHMM" for conflict naming
    pub stamp: &'a str,
}

pub fn plan(input: &PlanInput) -> Vec<Action> {
    let local: HashMap<String, &Entry> = input.local.iter().map(|e| (case_key(&e.path), e)).collect();
    let base: HashMap<String, Hash> = input.base.iter().map(|(p, h)| (case_key(p), *h)).collect();
    let mut remote: Vec<&Entry> = input.remote.iter().collect();
    remote.sort_by(|a, b| a.path.cmp(&b.path));

    let mut actions = Vec::new();
    for r in remote {
        // Remote deleted: do not propagate deletion
        let Some(rh) = r.hash else { continue };
        let key = case_key(&r.path);
        let b = base.get(&key).copied();
        let download = |local_path: &str, expected_local| Action::Download {
            local_path: local_path.to_string(),
            remote_path: r.path.clone(),
            hash: rh,
            expected_local,
        };
        match local.get(&key) {
            None => actions.push(download(&r.path, None)),
            Some(l) => match l.hash {
                None => {
                    if b != Some(rh) {
                        actions.push(download(&l.path, None));
                    }
                }
                Some(lh) if lh == rh => {
                    if b != Some(lh) {
                        actions.push(Action::UpdateBase { path: l.path.clone(), hash: lh });
                    }
                }
                Some(lh) if b == Some(lh) => actions.push(download(&l.path, Some(lh))),
                Some(_) if b == Some(rh) => {}
                Some(_) if input.i_win_conflicts => {}
                Some(_) => actions.push(Action::Conflict {
                    local_path: l.path.clone(),
                    conflict_path: conflict_name(&l.path, input.my_name, input.stamp),
                    remote_path: r.path.clone(),
                    hash: rh,
                }),
            },
        }
    }
    actions
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

    fn h(n: u8) -> Hash { [n; 32] }
    fn e(path: &str, hash: Option<u8>) -> Entry {
        Entry { path: path.into(), size: 1, mtime_ns: 1, hash: hash.map(h) }
    }
    fn run(local: &[Entry], remote: &[Entry], base: &[(&str, u8)], win: bool) -> Vec<Action> {
        let base: HashMap<String, Hash> = base.iter().map(|(p, n)| (p.to_string(), h(*n))).collect();
        plan(&PlanInput { local, remote, base: &base, i_win_conflicts: win, my_name: "Mac", stamp: "2026-10-13 1705" })
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
}
