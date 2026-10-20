//! Path normalization and file ignore rules.
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

fn strip_verbatim(p: &Path) -> &Path {
    let s = p.to_str().unwrap_or("");
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        Path::new(rest)
    } else {
        p
    }
}

/// Relative path from `root`, formatted as `a/b/c.txt`, NFC normalized.
/// Returns `None` if outside root, root itself, or non-UTF-8.
/// Safely handles Windows `\\?\` UNC prefixes, backslashes, and case-insensitive roots.
pub fn normalize_rel(root: &Path, full: &Path) -> Option<String> {
    let clean_root = strip_verbatim(root);
    let clean_full = strip_verbatim(full);

    let r_str = clean_root.to_str()?;
    let f_str = clean_full.to_str()?;

    let r_parts: Vec<&str> = r_str.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    let f_parts: Vec<&str> = f_str.split(['/', '\\']).filter(|s| !s.is_empty()).collect();

    if f_parts.len() <= r_parts.len() {
        return None;
    }

    for (r, f) in r_parts.iter().zip(f_parts.iter()) {
        if r != f && !r.eq_ignore_ascii_case(f) {
            return None;
        }
    }

    let mut parts = Vec::new();
    for part in &f_parts[r_parts.len()..] {
        if *part == "." || *part == ".." {
            return None;
        }
        parts.push(part.nfc().collect::<String>());
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

