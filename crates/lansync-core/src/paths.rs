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

/// Validates an untrusted relative path (e.g. from peer network message).
/// Rejects path traversal (`..`), absolute paths, Windows drive letters (`C:`), and null bytes.
/// Returns a safe, normalized NFC relative path with forward slashes, or `None` if dangerous.
pub fn validate_rel_path(rel: &str) -> Option<String> {
    if rel.is_empty() || rel.starts_with('/') || rel.starts_with('\\') || rel.contains('\0') {
        return None;
    }
    // Check for drive letter: e.g. "C:file" or "d:/folder"
    if rel.len() >= 2 && rel.as_bytes()[1] == b':' {
        return None;
    }
    let mut parts = Vec::new();
    for part in rel.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return None;
        }
        parts.push(part.nfc().collect::<String>());
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Converts relative path formatted as `a/b` to native filesystem path.
/// Safely sanitizes against path traversal by ensuring all parts are normal components.
pub fn to_fs_path(root: &Path, rel: &str) -> PathBuf {
    let safe_rel = validate_rel_path(rel).unwrap_or_else(|| {
        // Fallback: strip dangerous components
        rel.replace("..", "_").replace(':', "_")
    });
    let mut p = root.to_path_buf();
    for part in safe_rel.split('/') {
        if !part.is_empty() && part != "." && part != ".." {
            p.push(part);
        }
    }
    p
}

/// Case-insensitive key for path matching (standard APFS and NTFS behavior).
pub fn case_key(rel: &str) -> String {
    rel.nfc().collect::<String>().to_lowercase()
}

/// System files, temporary files, Office lock files, version archives: ignored from sync.
pub fn is_ignored(rel: &str) -> bool {
    rel.split('/').any(|name| {
        let lower = name.to_ascii_lowercase();
        name == ".DS_Store"
            || name == ".lansync-root"
            || name == ".lansync-versions"
            || name == ".lansyncignore"
            || name == "node_modules"
            || name == ".git"
            || lower == "thumbs.db"
            || lower == "desktop.ini"
            || name.starts_with("~$")
            || name.starts_with("._")
            || name.starts_with(".lansync-tmp-")
            || lower.ends_with(".tmp")
            || lower.ends_with(".part")
            || lower.ends_with(".crdownload")
    })
}

/// Matches gitignore-style ignore patterns from `.lansyncignore`.
pub fn matches_ignore_pattern(rel: &str, pattern: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() || p.starts_with('#') {
        return false;
    }
    let p = p.strip_prefix('/').unwrap_or(p);
    let p = p.strip_suffix('/').unwrap_or(p);

    if let Some(ext) = p.strip_prefix("*.") {
        return rel.ends_with(&format!(".{ext}"));
    }
    if let Some(suffix) = p.strip_prefix('*') {
        return rel.ends_with(suffix);
    }

    rel == p || rel.starts_with(&format!("{p}/")) || rel.contains(&format!("/{p}/")) || rel.ends_with(&format!("/{p}"))
}

/// Checks if path is ignored by default system rules or custom user patterns from `.lansyncignore`.
pub fn is_ignored_custom(rel: &str, custom_patterns: &[String]) -> bool {
    if is_ignored(rel) {
        return true;
    }
    custom_patterns.iter().any(|p| matches_ignore_pattern(rel, p))
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

/// Sanitizes a relative path for safe storage on Windows filesystems:
/// - Replaces invalid characters `< > : " | ? * \` with `_`.
/// - Strips trailing dots or spaces from path components.
/// - Appends `_renamed` if component is a reserved Windows device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`).
pub fn sanitize_for_windows(rel: &str) -> String {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let mut clean_parts = Vec::new();

    for name in rel.split('/') {
        let mut clean: String = name
            .chars()
            .map(|c| {
                if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') || (c as u32) < 32 {
                    '_'
                } else {
                    c
                }
            })
            .collect();

        while clean.ends_with('.') || clean.ends_with(' ') {
            clean.pop();
        }
        if clean.is_empty() {
            clean = "unnamed".to_string();
        }

        let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
        let com_lpt = (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9');

        if RESERVED.contains(&stem.as_str()) || com_lpt {
            if let Some(i) = clean.rfind('.') {
                clean = format!("{}_renamed{}", &clean[..i], &clean[i..]);
            } else {
                clean = format!("{clean}_renamed");
            }
        }
        clean_parts.push(clean);
    }
    clean_parts.join("/")
}

/// Resolves case collisions on case-insensitive filesystems (APFS, NTFS).
/// Generates an alternative name `file (case-conflict).ext` so both files coexist safely.
pub fn resolve_case_collision(path: &str) -> String {
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    format!("{dir}{stem} (case-conflict){ext}")
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

    #[test]
    fn validate_rel_path_rejects_traversal_and_escapes() {
        assert_eq!(validate_rel_path("../etc/passwd"), None);
        assert_eq!(validate_rel_path("sub/../../etc/passwd"), None);
        assert_eq!(validate_rel_path("/root/file.txt"), None);
        assert_eq!(validate_rel_path("\\Windows\\System32"), None);
        assert_eq!(validate_rel_path("C:autoexec.bat"), None);
        assert_eq!(validate_rel_path("D:\\data.txt"), None);
        assert_eq!(validate_rel_path("file\0.txt"), None);
        assert_eq!(validate_rel_path(""), None);

        assert_eq!(validate_rel_path("a/b/c.txt").as_deref(), Some("a/b/c.txt"));
        assert_eq!(validate_rel_path("docs\\note.md").as_deref(), Some("docs/note.md"));
    }

    #[test]
    fn test_sanitize_for_windows() {
        assert_eq!(sanitize_for_windows("a:b.txt"), "a_b.txt");
        assert_eq!(sanitize_for_windows("sub/file?.pdf"), "sub/file_.pdf");
        assert_eq!(sanitize_for_windows("CON.txt"), "CON_renamed.txt");
        assert_eq!(sanitize_for_windows("folder/ends-with-dot."), "folder/ends-with-dot");
        assert_eq!(sanitize_for_windows("AUX"), "AUX_renamed");
        assert_eq!(sanitize_for_windows("COM1.log"), "COM1_renamed.log");
    }

    #[test]
    fn test_resolve_case_collision() {
        assert_eq!(resolve_case_collision("Report.docx"), "Report (case-conflict).docx");
        assert_eq!(resolve_case_collision("sub/file.txt"), "sub/file (case-conflict).txt");
        assert_eq!(resolve_case_collision("Makefile"), "Makefile (case-conflict)");
    }
}
