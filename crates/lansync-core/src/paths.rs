//! Path normalization and file ignore rules.
use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

/// Relative path from `root`, formatted as `a/b/c.txt`, NFC normalized.
/// Returns `None` if outside root, root itself, or non-UTF-8.
pub fn normalize_rel(root: &Path, full: &Path) -> Option<String> {
    let rel = full.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?.nfc().collect::<String>()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Converts relative path formatted as `a/b` to native filesystem path.
pub fn to_fs_path(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
    }
    p
}

/// Case-insensitive key for path matching (standard APFS and NTFS behavior).
pub fn case_key(rel: &str) -> String {
    rel.nfc().collect::<String>().to_lowercase()
}

/// System files, temporary files, Office lock files: ignored from sync.
pub fn is_ignored(rel: &str) -> bool {
    rel.split('/').any(|name| {
        let lower = name.to_ascii_lowercase();
        name == ".DS_Store"
            || lower == "thumbs.db"
            || lower == "desktop.ini"
            || name.starts_with("~$")
            || name.starts_with("._")
            || name.starts_with(".lansync-tmp-")
            || lower.ends_with(".tmp")
    })
}

/// Reason a filename is invalid on Windows, or `None` if valid.
pub fn windows_invalid_reason(rel: &str) -> Option<String> {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    for name in rel.split('/') {
        if let Some(ch) = name
            .chars()
            .find(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') || (*c as u32) < 32)
        {
            return Some(format!("name '{name}' contains character {ch:?} invalid on Windows"));
        }
        if name.ends_with('.') || name.ends_with(' ') {
            return Some(format!("name '{name}' ends with dot or whitespace"));
        }
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let com_lpt = (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9');
        if RESERVED.contains(&stem.as_str()) || com_lpt {
            return Some(format!("name '{name}' is a reserved Windows device name"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn normalize_rel_uses_slash_and_nfc() {
        let root = Path::new("/tmp/root");
        // Unicode string in NFD form (e + combining circumflex + combining acute) commonly returned by macOS
        let full = root.join("folder").join("Tie\u{0302}\u{0301}ng.txt");
        assert_eq!(normalize_rel(root, &full).as_deref(), Some("folder/Ti\u{1EBF}ng.txt"));
    }

    #[test]
    fn normalize_rel_rejects_outside_root_and_root_itself() {
        let root = Path::new("/tmp/root");
        assert_eq!(normalize_rel(root, Path::new("/tmp/other/a.txt")), None);
        assert_eq!(normalize_rel(root, root), None);
    }

    #[test]
    fn to_fs_path_roundtrip() {
        let root = Path::new("/tmp/root");
        let p = to_fs_path(root, "a/b/c.txt");
        assert_eq!(normalize_rel(root, &p).as_deref(), Some("a/b/c.txt"));
    }

    #[test]
    fn case_key_is_case_insensitive() {
        assert_eq!(case_key("Docs/Report.TXT"), case_key("docs/report.txt"));
    }

    #[test]
    fn ignore_rules() {
        for p in [".DS_Store", "a/.DS_Store", "Thumbs.db", "desktop.ini", "~$report.docx",
                  "x.tmp", "X.TMP", "sub/.lansync-tmp-abc", "._file"] {
            assert!(is_ignored(p), "should ignore {p}");
        }
        for p in ["a.txt", "tmp/a.txt", "report.docx", ".env"] {
            assert!(!is_ignored(p), "should not ignore {p}");
        }
    }

    #[test]
    fn windows_invalid_names() {
        for p in ["a:b.txt", "sub/what?.txt", "CON", "con.txt", "COM1.log", "lpt9", "ends-with-dot.", "space "] {
            assert!(windows_invalid_reason(p).is_some(), "{p} should be invalid on Windows");
        }
        for p in ["COM10.txt", "console.txt", "a/b.txt", "Report.docx"] {
            assert!(windows_invalid_reason(p).is_none(), "{p} should be valid on Windows");
        }
    }
}
