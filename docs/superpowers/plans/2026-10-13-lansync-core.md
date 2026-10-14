# LanSync Core — Implementation Plan (Plan 1/2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Xây crate `lansync-core`, phần lõi đồng bộ 2 chiều thư mục `LanSync` giữa 2 máy qua LAN (ghép cặp bằng mã 6 số, mTLS, không đồng bộ việc xoá, xung đột thì giữ cả hai bản). Kèm example `dev` để test tay giữa Mac và Windows.

**Architecture:** Workspace Cargo có một crate thư viện `crates/lansync-core`. Mỗi module có một nhiệm vụ: `paths` (chuẩn hoá tên), `protocol` (message và frame), `index` (SQLite), `planner` (hàm thuần quyết định tải gì), `identity`/`pairing` (chứng chỉ, mã ghép cặp), `transport` (TLS), `transfer` (gửi/nhận file), `watcher`, `discovery` (mDNS), `engine` (nối tất cả lại). Engine chạy trên tokio. Mỗi phiên kết nối có 4 task: reader, writer, puller, pinger.

**Tech Stack:** Rust 1.98 (edition 2024), tokio, rustls 0.23 (ring) + tokio-rustls 0.26, rcgen 0.13, rusqlite (bundled), blake3, bincode 1, notify 8 + notify-debouncer-full 0.5, mdns-sd 0.13.

**Spec:** [`docs/superpowers/specs/2026-10-13-lansync-design.md`](../specs/2026-10-13-lansync-design.md)

## Global Constraints

- Rust stable **1.98**, edition **2024**. Workspace `resolver = "3"`.
- Nền tảng: **macOS 13+** (Apple Silicon + Intel), **Windows 10/11 x64**. Không dùng API riêng của một hệ điều hành, trừ khi có `#[cfg]`.
- TLS **chỉ 1.3**, xác thực hai chiều (mTLS), crypto provider **ring** (không dùng aws-lc-rs vì build trên Windows cần cmake/nasm).
- Cổng mặc định **47800**. mDNS service `_lansync._tcp.local.`, TXT `id`, `name`, `v=1`.
- `PROTO_VERSION = 1`. Frame = `u32 BE` độ dài + bincode. Frame tối đa **16 MiB**. Mỗi chunk tối đa **256 KiB**. Mỗi frame index tối đa **5.000** entry.
- Đường dẫn trong index/giao thức: `/`, Unicode NFC. So sánh không phân biệt hoa thường.
- Dữ liệu app (index, chứng chỉ, `peers.json`) **không** nằm trong thư mục đồng bộ.
- **Không đồng bộ việc xoá.** Xung đột: máy có `device_id` nhỏ hơn giữ tên gốc. Máy kia đổi tên bản của mình thành `tên (conflict-<tên máy> YYYY-MM-DD HHMM).ext`.
- Comment trong code viết bằng tiếng Việt.
- Thư mục gốc dự án: `/Users/keith/Documents/TRANFER_FILE`. Mọi lệnh bên dưới chạy từ thư mục này.

---

## File Structure

```
TRANFER_FILE/
├── .gitignore
├── Cargo.toml                         # workspace
├── README.md                          # (Task 11)
└── crates/lansync-core/
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs                     # khai báo module + re-export
    │   ├── paths.rs                   # Task 1
    │   ├── protocol.rs                # Task 2
    │   ├── fsutil.rs                  # Task 2 (hash_file, mtime)
    │   ├── index.rs                   # Task 3
    │   ├── planner.rs                 # Task 4
    │   ├── identity.rs                # Task 5
    │   ├── pairing.rs                 # Task 5
    │   ├── transport.rs               # Task 6
    │   ├── transfer.rs                # Task 7
    │   ├── watcher.rs                 # Task 8
    │   ├── discovery.rs               # Task 8
    │   ├── config.rs                  # Task 9
    │   └── engine.rs                  # Task 9
    ├── tests/
    │   └── sync.rs                    # Task 9, 10 (integration)
    └── examples/
        └── dev.rs                     # Task 11
```

---

### Task 1: Workspace + chuẩn hoá đường dẫn (`paths.rs`)

**Files:**
- Create: `.gitignore`, `Cargo.toml`, `crates/lansync-core/Cargo.toml`, `crates/lansync-core/src/lib.rs`, `crates/lansync-core/src/paths.rs`

**Interfaces:**
- Produces:
  - `paths::normalize_rel(root: &Path, full: &Path) -> Option<String>`
  - `paths::to_fs_path(root: &Path, rel: &str) -> PathBuf`
  - `paths::case_key(rel: &str) -> String`
  - `paths::is_ignored(rel: &str) -> bool`
  - `paths::windows_invalid_reason(rel: &str) -> Option<String>`

- [x] **Step 1: Tạo workspace**

`.gitignore`:
```gitignore
/target
**/*.rs.bk
.DS_Store
```

`Cargo.toml`:
```toml
[workspace]
resolver = "3"
members = ["crates/lansync-core"]

[workspace.package]
edition = "2024"
rust-version = "1.98"
```

`crates/lansync-core/Cargo.toml` (khai báo đủ dependency cho mọi task ngay từ đầu để các task sau không phải sửa file này):
```toml
[package]
name = "lansync-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1.3"
blake3 = "1"
sha2 = "0.10"
data-encoding = "2"
unicode-normalization = "0.1"
rusqlite = { version = "0.32", features = ["bundled"] }
walkdir = "2"
filetime = "0.2"
rand = "0.8"
chrono = "0.4"
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"
rustls = { version = "0.23", default-features = false, features = ["ring", "std", "logging"] }
tokio-rustls = { version = "0.26", default-features = false, features = ["ring", "logging"] }
rcgen = "0.13"
mdns-sd = "0.13"
notify = "8"
notify-debouncer-full = "0.5"
directories = "6"
hostname = "0.4"
tracing = "0.1"

[dev-dependencies]
tempfile = "3"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

`crates/lansync-core/src/lib.rs`:
```rust
//! LanSync core: đồng bộ thư mục 2 chiều qua mạng LAN.
pub mod paths;
```

- [x] **Step 2: Viết test (failing)**

`crates/lansync-core/src/paths.rs`:
```rust
//! Chuẩn hoá đường dẫn và luật bỏ qua file.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn normalize_rel_uses_slash_and_nfc() {
        let root = Path::new("/tmp/root");
        // "Tiếng" viết dạng NFD (e + dấu mũ + dấu sắc) như macOS hay trả về
        let full = root.join("thu muc").join("Tie\u{0302}\u{0301}ng.txt");
        assert_eq!(normalize_rel(root, &full).as_deref(), Some("thu muc/Ti\u{1EBF}ng.txt"));
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
        for p in [".DS_Store", "a/.DS_Store", "Thumbs.db", "desktop.ini", "~$bao-cao.docx",
                  "x.tmp", "X.TMP", "sub/.lansync-tmp-abc", "._file"] {
            assert!(is_ignored(p), "phải bỏ qua {p}");
        }
        for p in ["a.txt", "tmp/a.txt", "bao-cao.docx", ".env"] {
            assert!(!is_ignored(p), "không được bỏ qua {p}");
        }
    }

    #[test]
    fn windows_invalid_names() {
        for p in ["a:b.txt", "sub/what?.txt", "CON", "con.txt", "COM1.log", "lpt9", "ends-with-dot.", "space "] {
            assert!(windows_invalid_reason(p).is_some(), "{p} phải không hợp lệ");
        }
        for p in ["COM10.txt", "console.txt", "a/b.txt", "Tiếng Việt.docx"] {
            assert!(windows_invalid_reason(p).is_none(), "{p} phải hợp lệ");
        }
    }
}
```

- [x] **Step 3: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core paths`
Expected: lỗi biên dịch `cannot find function normalize_rel`.

- [x] **Step 4: Implement**

Thêm vào đầu `paths.rs` (phía trên `mod tests`):
```rust
use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

/// Đường dẫn tương đối so với `root`, dạng `a/b/c.txt`, Unicode NFC.
/// Trả `None` nếu nằm ngoài root, là chính root, hoặc tên không phải UTF-8.
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

/// Đổi đường dẫn tương đối dạng `a/b` thành đường dẫn thật trên ổ đĩa.
pub fn to_fs_path(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
    }
    p
}

/// Khoá so sánh không phân biệt hoa thường (APFS và NTFS mặc định đều như vậy).
pub fn case_key(rel: &str) -> String {
    rel.nfc().collect::<String>().to_lowercase()
}

/// File hệ thống, file tạm, file khoá của Office: không đồng bộ.
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

/// Lý do tên không dùng được trên Windows, hoặc `None` nếu hợp lệ.
pub fn windows_invalid_reason(rel: &str) -> Option<String> {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    for name in rel.split('/') {
        if let Some(ch) = name
            .chars()
            .find(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') || (*c as u32) < 32)
        {
            return Some(format!("tên '{name}' chứa ký tự {ch:?} không hợp lệ trên Windows"));
        }
        if name.ends_with('.') || name.ends_with(' ') {
            return Some(format!("tên '{name}' kết thúc bằng dấu chấm hoặc khoảng trắng"));
        }
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let com_lpt = (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9');
        if RESERVED.contains(&stem.as_str()) || com_lpt {
            return Some(format!("tên '{name}' là tên dành riêng của Windows"));
        }
    }
    None
}
```

- [x] **Step 5: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core paths`
Expected: `6 passed`.

- [x] **Step 6: Commit**

```bash
git add .gitignore Cargo.toml crates/lansync-core
git commit -m "feat(core): workspace + chuẩn hoá đường dẫn và luật bỏ qua"
```

---

### Task 2: Giao thức + tiện ích file (`protocol.rs`, `fsutil.rs`)

**Files:**
- Create: `crates/lansync-core/src/protocol.rs`, `crates/lansync-core/src/fsutil.rs`
- Modify: `crates/lansync-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `protocol::{Hash = [u8; 32], Entry, Message, PROTO_VERSION, MAX_FRAME, CHUNK_SIZE, INDEX_BATCH}`
  - `protocol::write_msg<W: AsyncWrite + Unpin>(&mut W, &Message) -> anyhow::Result<()>`
  - `protocol::read_msg<R: AsyncRead + Unpin>(&mut R) -> anyhow::Result<Message>`
  - `protocol::index_messages(entries: Vec<Entry>, full: bool) -> Vec<Message>`
  - `fsutil::hash_file(&Path) -> io::Result<Hash>`, `fsutil::mtime_ns(&Metadata) -> i64`, `fsutil::filetime_from_ns(i64) -> filetime::FileTime`

- [x] **Step 1: Khai báo module**

`lib.rs`, thêm:
```rust
pub mod fsutil;
pub mod protocol;
```

- [x] **Step 2: Viết test (failing)**

`crates/lansync-core/src/protocol.rs`:
```rust
//! Message trao đổi giữa 2 máy và cách đóng gói frame.

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn entry(n: usize) -> Entry {
        Entry { path: format!("f{n}.txt"), size: n as u64, mtime_ns: 1, hash: Some([n as u8; 32]) }
    }

    #[tokio::test]
    async fn frame_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let msgs = vec![
            Message::Hello { device_id: "ID".into(), name: "Mac".into(), proto: PROTO_VERSION, pairing: false, listen_port: 47800 },
            Message::Index { entries: vec![entry(1), Entry { hash: None, ..entry(2) }] },
            Message::FileChunk { data: vec![7; 1000] },
            Message::Ping,
        ];
        for m in &msgs {
            write_msg(&mut a, m).await.unwrap();
        }
        for m in &msgs {
            assert_eq!(&read_msg(&mut b).await.unwrap(), m);
        }
    }

    #[tokio::test]
    async fn rejects_oversized_frame() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_u32(MAX_FRAME + 1).await.unwrap();
        let err = read_msg(&mut b).await.unwrap_err();
        assert!(err.to_string().contains("frame quá lớn"), "{err}");
    }

    #[test]
    fn index_messages_batches() {
        let entries: Vec<Entry> = (0..12_001).map(entry).collect();
        let msgs = index_messages(entries, true);
        assert_eq!(msgs.len(), 3);
        assert!(matches!(&msgs[0], Message::Index { entries } if entries.len() == INDEX_BATCH));
        assert!(matches!(&msgs[2], Message::IndexUpdate { entries } if entries.len() == 2_001));
        // index rỗng vẫn phải gửi 1 frame Index để máy kia biết đã nhận đủ
        assert_eq!(index_messages(vec![], true), vec![Message::Index { entries: vec![] }]);
        assert!(index_messages(vec![], false).is_empty());
    }
}
```

`crates/lansync-core/src/fsutil.rs`:
```rust
//! Tiện ích file: hash blake3, mtime theo nano giây.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_blake3() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"xin chao").unwrap();
        assert_eq!(hash_file(&p).unwrap(), *blake3::hash(b"xin chao").as_bytes());
    }

    #[test]
    fn mtime_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"x").unwrap();
        let ns: i64 = 1_700_000_000_123_456_700; // bội số 100ns để khớp độ phân giải NTFS
        filetime::set_file_mtime(&p, filetime_from_ns(ns)).unwrap();
        assert_eq!(mtime_ns(&std::fs::metadata(&p).unwrap()), ns);
    }
}
```

- [x] **Step 3: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core protocol fsutil`
Expected: lỗi biên dịch (thiếu `Entry`, `hash_file`...).

> Lưu ý: `cargo test` chỉ nhận một filter. Hãy chạy `cargo test -p lansync-core protocol` rồi `cargo test -p lansync-core fsutil`.

- [x] **Step 4: Implement**

Đầu `protocol.rs`:
```rust
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTO_VERSION: u16 = 1;
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;
pub const CHUNK_SIZE: usize = 256 * 1024;
pub const INDEX_BATCH: usize = 5_000;

pub type Hash = [u8; 32];

/// Một file trong index. `hash = None` nghĩa là máy đó đã xoá file cục bộ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub hash: Option<Hash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello { device_id: String, name: String, proto: u16, pairing: bool, listen_port: u16 },
    PairDecision { accept: bool },
    Index { entries: Vec<Entry> },
    IndexUpdate { entries: Vec<Entry> },
    GetFile { path: String, hash: Hash },
    FileHeader { path: String, size: u64, hash: Hash, mtime_ns: i64 },
    FileChunk { data: Vec<u8> },
    FileEnd,
    FileUnavailable { path: String },
    Ping,
    Pong,
}

pub async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> Result<()> {
    let buf = bincode::serialize(msg)?;
    ensure!(buf.len() as u64 <= MAX_FRAME as u64, "frame quá lớn: {} byte", buf.len());
    w.write_u32(buf.len() as u32).await?;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> Result<Message> {
    let len = r.read_u32().await?;
    ensure!(len <= MAX_FRAME, "frame quá lớn: {len} byte");
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(bincode::deserialize(&buf)?)
}

/// Chia index thành nhiều frame. `full = true`: frame đầu là `Index` (thay toàn bộ),
/// các frame sau là `IndexUpdate`.
pub fn index_messages(entries: Vec<Entry>, full: bool) -> Vec<Message> {
    let mut chunks = entries.chunks(INDEX_BATCH).map(|c| c.to_vec());
    let mut out = Vec::new();
    if full {
        out.push(Message::Index { entries: chunks.next().unwrap_or_default() });
    }
    out.extend(chunks.map(|entries| Message::IndexUpdate { entries }));
    out
}
```

Đầu `fsutil.rs`:
```rust
use crate::protocol::Hash;
use filetime::FileTime;
use std::fs::{File, Metadata};
use std::path::Path;
use std::time::UNIX_EPOCH;

pub fn hash_file(path: &Path) -> std::io::Result<Hash> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(*h.finalize().as_bytes())
}

pub fn mtime_ns(meta: &Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

pub fn filetime_from_ns(ns: i64) -> FileTime {
    FileTime::from_unix_time(ns.div_euclid(1_000_000_000), ns.rem_euclid(1_000_000_000) as u32)
}
```

- [x] **Step 5: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core protocol && cargo test -p lansync-core fsutil`
Expected: `3 passed` và `2 passed`.

- [x] **Step 6: Commit**

```bash
git add crates/lansync-core
git commit -m "feat(core): định nghĩa message, frame và tiện ích file"
```

---

### Task 3: Index SQLite (`index.rs`)

**Files:**
- Create: `crates/lansync-core/src/index.rs`
- Modify: `crates/lansync-core/src/lib.rs` (thêm `pub mod index;`)

**Interfaces:**
- Consumes: `paths::{normalize_rel, is_ignored, windows_invalid_reason}`, `fsutil::{hash_file, mtime_ns}`, `protocol::{Entry, Hash}`
- Produces:
  - `index::ScanReport { changed: Vec<Entry>, skipped: Vec<(String, String)> }`
  - `index::Index::open(db_path: &Path, root: &Path) -> Result<Index>`
  - `Index::scan(&mut self) -> Result<ScanReport>`
  - `Index::entries(&self) -> Result<Vec<Entry>>` (gồm cả entry đã xoá với `hash: None`)
  - `Index::get(&self, path: &str) -> Result<Option<Entry>>`
  - `Index::record_file(&mut self, e: &Entry) -> Result<()>`
  - `Index::bases(&self, peer_id: &str) -> Result<HashMap<String, Hash>>`
  - `Index::set_base(&mut self, peer_id: &str, path: &str, hash: &Hash) -> Result<()>`

- [x] **Step 1: Viết test (failing)**

`crates/lansync-core/src/index.rs`:
```rust
//! Index: danh sách file + hash, lưu trong SQLite.

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PathBuf, Index) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("sync");
        fs::create_dir_all(&root).unwrap();
        let idx = Index::open(&t.path().join("index.db"), &root).unwrap();
        (t, root, idx)
    }

    fn paths(r: &ScanReport) -> Vec<String> {
        let mut v: Vec<String> = r.changed.iter().map(|e| e.path.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn scan_finds_new_files_and_skips_ignored() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a.txt"), b"aaa").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.txt"), b"bb").unwrap();
        fs::write(root.join(".DS_Store"), b"x").unwrap();
        let r = idx.scan().unwrap();
        assert_eq!(paths(&r), vec!["a.txt", "sub/b.txt"]);
        let a = idx.get("a.txt").unwrap().unwrap();
        assert_eq!(a.hash, Some(*blake3::hash(b"aaa").as_bytes()));
        assert_eq!(a.size, 3);
    }

    #[test]
    fn second_scan_without_changes_reports_nothing() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a.txt"), b"aaa").unwrap();
        idx.scan().unwrap();
        assert!(idx.scan().unwrap().changed.is_empty());
    }

    #[test]
    fn modified_file_gets_new_hash() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a.txt"), b"1").unwrap();
        idx.scan().unwrap();
        fs::write(root.join("a.txt"), b"22").unwrap();
        let r = idx.scan().unwrap();
        assert_eq!(r.changed.len(), 1);
        assert_eq!(r.changed[0].hash, Some(*blake3::hash(b"22").as_bytes()));
    }

    #[test]
    fn deleted_file_is_kept_with_none_hash() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a.txt"), b"1").unwrap();
        idx.scan().unwrap();
        fs::remove_file(root.join("a.txt")).unwrap();
        let r = idx.scan().unwrap();
        assert_eq!(r.changed.len(), 1);
        assert_eq!(r.changed[0].hash, None);
        assert_eq!(idx.entries().unwrap()[0].hash, None);
        // quét lần nữa không báo lại
        assert!(idx.scan().unwrap().changed.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn windows_invalid_names_are_skipped() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a:b.txt"), b"1").unwrap();
        let r = idx.scan().unwrap();
        assert!(r.changed.is_empty());
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].0, "a:b.txt");
    }

    #[test]
    fn record_file_and_bases() {
        let (_t, _root, mut idx) = setup();
        let e = Entry { path: "x.txt".into(), size: 5, mtime_ns: 9, hash: Some([3; 32]) };
        idx.record_file(&e).unwrap();
        assert_eq!(idx.get("x.txt").unwrap(), Some(e));
        idx.set_base("PEER", "x.txt", &[1; 32]).unwrap();
        idx.set_base("PEER", "x.txt", &[2; 32]).unwrap();
        assert_eq!(idx.bases("PEER").unwrap().get("x.txt"), Some(&[2; 32]));
        assert!(idx.bases("OTHER").unwrap().is_empty());
    }
}
```

- [x] **Step 2: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core index`
Expected: lỗi biên dịch `cannot find type Index`.

- [x] **Step 3: Implement**

Đầu `index.rs`:
```rust
use crate::fsutil::{hash_file, mtime_ns};
use crate::paths::{is_ignored, normalize_rel, windows_invalid_reason};
use crate::protocol::{Entry, Hash};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub struct ScanReport {
    /// File mới, file bị sửa, file bị xoá (hash = None)
    pub changed: Vec<Entry>,
    /// (đường dẫn, lý do) các file không đồng bộ được
    pub skipped: Vec<(String, String)>,
}

pub struct Index {
    conn: Connection,
    root: PathBuf,
}

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS files (
  path     TEXT PRIMARY KEY,
  size     INTEGER NOT NULL,
  mtime_ns INTEGER NOT NULL,
  hash     BLOB,
  deleted  INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS peer_base (
  peer_id   TEXT NOT NULL,
  path      TEXT NOT NULL,
  base_hash BLOB NOT NULL,
  PRIMARY KEY (peer_id, path)
);";

fn row_to_entry(row: &Row) -> rusqlite::Result<Entry> {
    let hash: Option<Vec<u8>> = row.get(3)?;
    Ok(Entry {
        path: row.get(0)?,
        size: row.get::<_, i64>(1)? as u64,
        mtime_ns: row.get(2)?,
        hash: hash.and_then(|v| v.try_into().ok()),
    })
}

impl Index {
    pub fn open(db_path: &Path, root: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn, root: root.to_path_buf() })
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        let mut st = self.conn.prepare("SELECT path, size, mtime_ns, hash FROM files ORDER BY path")?;
        let rows = st.query_map([], row_to_entry)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get(&self, path: &str) -> Result<Option<Entry>> {
        Ok(self
            .conn
            .query_row("SELECT path, size, mtime_ns, hash FROM files WHERE path = ?1", [path], row_to_entry)
            .optional()?)
    }

    pub fn record_file(&mut self, e: &Entry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO files (path, size, mtime_ns, hash, deleted) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime_ns = excluded.mtime_ns,
               hash = excluded.hash, deleted = excluded.deleted",
            params![e.path, e.size as i64, e.mtime_ns, e.hash.map(|h| h.to_vec()), e.hash.is_none() as i64],
        )?;
        Ok(())
    }

    pub fn bases(&self, peer_id: &str) -> Result<HashMap<String, Hash>> {
        let mut st = self.conn.prepare("SELECT path, base_hash FROM peer_base WHERE peer_id = ?1")?;
        let rows = st
            .query_map([peer_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter_map(|(p, h)| h.try_into().ok().map(|h| (p, h))).collect())
    }

    pub fn set_base(&mut self, peer_id: &str, path: &str, hash: &Hash) -> Result<()> {
        self.conn.execute(
            "INSERT INTO peer_base (peer_id, path, base_hash) VALUES (?1, ?2, ?3)
             ON CONFLICT(peer_id, path) DO UPDATE SET base_hash = excluded.base_hash",
            params![peer_id, path, hash.to_vec()],
        )?;
        Ok(())
    }

    /// Quét thư mục. Chỉ hash lại file có size/mtime thay đổi.
    pub fn scan(&mut self) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        let known: HashMap<String, Entry> = self.entries()?.into_iter().map(|e| (e.path.clone(), e)).collect();
        let mut seen = HashSet::new();

        for item in WalkDir::new(&self.root).follow_links(false).into_iter().filter_map(|r| r.ok()) {
            // bỏ qua thư mục và symlink
            if !item.file_type().is_file() {
                continue;
            }
            let Some(rel) = normalize_rel(&self.root, item.path()) else { continue };
            if is_ignored(&rel) {
                continue;
            }
            if let Some(reason) = windows_invalid_reason(&rel) {
                report.skipped.push((rel, reason));
                continue;
            }
            let Ok(meta) = item.metadata() else { continue };
            let (size, mtime) = (meta.len(), mtime_ns(&meta));
            seen.insert(rel.clone());
            if let Some(old) = known.get(&rel) {
                if old.hash.is_some() && old.size == size && old.mtime_ns == mtime {
                    continue;
                }
            }
            let hash = match hash_file(item.path()) {
                Ok(h) => h,
                Err(err) => {
                    tracing::warn!("không đọc được {rel}: {err}");
                    continue;
                }
            };
            // file đang được ghi dở (size/mtime đổi trong lúc hash) → để lần quét sau
            match std::fs::metadata(item.path()) {
                Ok(after) if after.len() == size && mtime_ns(&after) == mtime => {}
                _ => continue,
            }
            let e = Entry { path: rel, size, mtime_ns: mtime, hash: Some(hash) };
            self.record_file(&e)?;
            report.changed.push(e);
        }

        for (path, old) in known {
            if old.hash.is_some() && !seen.contains(&path) {
                let e = Entry { path, size: 0, mtime_ns: 0, hash: None };
                self.record_file(&e)?;
                report.changed.push(e);
            }
        }
        Ok(report)
    }
}
```

- [x] **Step 4: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core index`
Expected: `6 passed` (5 trên Windows vì test tên `a:b.txt` chỉ chạy trên unix).

- [x] **Step 5: Commit**

```bash
git add crates/lansync-core
git commit -m "feat(core): index SQLite với quét thư mục và base hash"
```

---

### Task 4: Planner (`planner.rs`) — hàm thuần

**Files:**
- Create: `crates/lansync-core/src/planner.rs`
- Modify: `crates/lansync-core/src/lib.rs` (thêm `pub mod planner;`)

**Interfaces:**
- Consumes: `paths::case_key`, `protocol::{Entry, Hash}`
- Produces:
  - `planner::Action::{Download { local_path, remote_path, hash, expected_local: Option<Hash> }, UpdateBase { path, hash }, Conflict { local_path, conflict_path, remote_path, hash }}`
  - `planner::PlanInput<'a> { local: &'a [Entry], remote: &'a [Entry], base: &'a HashMap<String, Hash>, i_win_conflicts: bool, my_name: &'a str, stamp: &'a str }`
  - `planner::plan(&PlanInput) -> Vec<Action>`
  - `planner::conflict_name(path: &str, device_name: &str, stamp: &str) -> String`

- [x] **Step 1: Viết test (failing)**. Mỗi dòng trong bảng 5.8 của spec có ít nhất 1 test.

`crates/lansync-core/src/planner.rs`:
```rust
//! Quyết định cần tải gì từ máy kia. Hàm thuần, không I/O.

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
        let local = [e("docs/bao-cao.docx", Some(2))];
        let remote = [e("docs/bao-cao.docx", Some(3))];
        assert_eq!(run(&local, &remote, &[("docs/bao-cao.docx", 1)], false), vec![Action::Conflict {
            local_path: "docs/bao-cao.docx".into(),
            conflict_path: "docs/bao-cao (conflict-Mac 2026-10-13 1705).docx".into(),
            remote_path: "docs/bao-cao.docx".into(),
            hash: h(3),
        }]);
        assert!(run(&local, &remote, &[("docs/bao-cao.docx", 1)], true).is_empty());
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
```

- [x] **Step 2: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core planner`
Expected: lỗi biên dịch `cannot find type Action`.

- [x] **Step 3: Implement**

Đầu `planner.rs`:
```rust
use crate::paths::case_key;
use crate::protocol::{Entry, Hash};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Tải `remote_path` của máy kia về `local_path`. `expected_local` là hash bản ở máy mình
    /// lúc lập kế hoạch (None = chưa có/đã xoá), dùng để không ghi đè nếu người dùng vừa sửa.
    Download { local_path: String, remote_path: String, hash: Hash, expected_local: Option<Hash> },
    /// Hai bên đã giống nhau, chỉ cập nhật base.
    UpdateBase { path: String, hash: Hash },
    /// Xung đột, máy mình "thua": đổi tên bản của mình rồi tải bản kia về đúng tên gốc.
    Conflict { local_path: String, conflict_path: String, remote_path: String, hash: Hash },
}

pub struct PlanInput<'a> {
    pub local: &'a [Entry],
    pub remote: &'a [Entry],
    /// base hash theo path (của cặp máy này)
    pub base: &'a HashMap<String, Hash>,
    /// true nếu device_id của mình nhỏ hơn máy kia
    pub i_win_conflicts: bool,
    pub my_name: &'a str,
    /// thời điểm dạng "YYYY-MM-DD HHMM" để đặt tên file xung đột
    pub stamp: &'a str,
}

pub fn plan(input: &PlanInput) -> Vec<Action> {
    let local: HashMap<String, &Entry> = input.local.iter().map(|e| (case_key(&e.path), e)).collect();
    let base: HashMap<String, Hash> = input.base.iter().map(|(p, h)| (case_key(p), *h)).collect();
    let mut remote: Vec<&Entry> = input.remote.iter().collect();
    remote.sort_by(|a, b| a.path.cmp(&b.path));

    let mut actions = Vec::new();
    for r in remote {
        // máy kia đã xoá: không đồng bộ việc xoá
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

/// `dir/ten.ext` → `dir/ten (conflict-<máy> <stamp>).ext`
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
```

- [x] **Step 4: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core planner`
Expected: `11 passed`.

- [x] **Step 5: Commit**

```bash
git add crates/lansync-core
git commit -m "feat(core): planner pull-only với xử lý xung đột"
```

---

### Task 5: Danh tính + ghép cặp (`identity.rs`, `pairing.rs`)

**Files:**
- Create: `crates/lansync-core/src/identity.rs`, `crates/lansync-core/src/pairing.rs`
- Modify: `crates/lansync-core/src/lib.rs` (thêm `pub mod identity; pub mod pairing;`)

**Interfaces:**
- Produces:
  - `identity::Identity { cert_der: Vec<u8>, key_der: Vec<u8>, device_id: String }`
  - `Identity::generate() -> Result<Identity>`, `Identity::load_or_create(dir: &Path) -> Result<Identity>`
  - `identity::device_id_from_cert(&[u8]) -> String`, `identity::fingerprint(&[u8]) -> String` (hex SHA-256)
  - `pairing::pairing_code(cert_a: &[u8], cert_b: &[u8]) -> String` (6 chữ số)
  - `pairing::Peer { device_id, name, cert_fingerprint, last_addr: Option<String> }` (Serialize, Deserialize, Clone, Debug, PartialEq)
  - `pairing::PeerStore::{load(&Path) -> Result<Self>, save(&self) -> Result<()>, list(&self) -> &[Peer], add(&mut self, Peer), remove(&mut self, id: &str) -> bool, find_by_fingerprint(&self, fp: &str) -> Option<&Peer>, set_addr(&mut self, id: &str, addr: &str)}`

- [x] **Step 1: Viết test (failing)**

`crates/lansync-core/src/identity.rs`:
```rust
//! Chứng chỉ tự ký của máy và device_id.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_16_base32_chars() {
        let id = Identity::generate().unwrap();
        assert_eq!(id.device_id.len(), 16);
        assert!(id.device_id.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
        assert_eq!(id.device_id, device_id_from_cert(&id.cert_der));
    }

    #[test]
    fn load_or_create_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let a = Identity::load_or_create(dir.path()).unwrap();
        let b = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(a.device_id, b.device_id);
        assert_eq!(a.cert_der, b.cert_der);
        assert_ne!(a.device_id, Identity::generate().unwrap().device_id);
    }

    #[test]
    fn fingerprint_is_hex_sha256() {
        assert_eq!(fingerprint(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
```

`crates/lansync-core/src/pairing.rs`:
```rust
//! Mã ghép cặp 6 số và danh sách máy đã ghép (peers.json).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_symmetric_six_digits_and_cert_dependent() {
        let c1 = pairing_code(b"cert-mac", b"cert-win");
        assert_eq!(c1, pairing_code(b"cert-win", b"cert-mac"));
        assert_eq!(c1.len(), 6);
        assert!(c1.chars().all(|c| c.is_ascii_digit()));
        assert_ne!(c1, pairing_code(b"cert-mac", b"cert-attacker"));
    }

    fn peer(id: &str, fp: &str) -> Peer {
        Peer { device_id: id.into(), name: format!("May {id}"), cert_fingerprint: fp.into(), last_addr: None }
    }

    #[test]
    fn store_persists_and_replaces_same_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let mut s = PeerStore::load(&path).unwrap();
        assert!(s.list().is_empty());
        s.add(peer("A", "fp1"));
        s.add(peer("B", "fp2"));
        s.add(peer("A", "fp3"));           // ghép lại → thay bản cũ
        s.set_addr("B", "192.168.1.5:47800");
        s.save().unwrap();

        let s2 = PeerStore::load(&path).unwrap();
        assert_eq!(s2.list().len(), 2);
        assert_eq!(s2.find_by_fingerprint("fp3").unwrap().device_id, "A");
        assert!(s2.find_by_fingerprint("fp1").is_none());
        assert_eq!(s2.find_by_fingerprint("fp2").unwrap().last_addr.as_deref(), Some("192.168.1.5:47800"));
    }

    #[test]
    fn remove_peer() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = PeerStore::load(&dir.path().join("peers.json")).unwrap();
        s.add(peer("A", "fp1"));
        assert!(s.remove("A"));
        assert!(!s.remove("A"));
        assert!(s.list().is_empty());
    }
}
```

- [x] **Step 2: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core identity && cargo test -p lansync-core pairing`
Expected: lỗi biên dịch.

- [x] **Step 3: Implement**

Đầu `identity.rs`:
```rust
use anyhow::Result;
use data_encoding::{BASE32_NOPAD, HEXLOWER};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct Identity {
    pub cert_der: Vec<u8>,
    /// private key PKCS#8 DER
    pub key_der: Vec<u8>,
    pub device_id: String,
}

pub fn device_id_from_cert(cert_der: &[u8]) -> String {
    BASE32_NOPAD.encode(&Sha256::digest(cert_der))[..16].to_string()
}

pub fn fingerprint(cert_der: &[u8]) -> String {
    HEXLOWER.encode(&Sha256::digest(cert_der))
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
        let params = rcgen::CertificateParams::new(vec!["lansync.local".to_string()])?;
        let cert = params.self_signed(&key)?;
        let cert_der = cert.der().to_vec();
        Ok(Self { device_id: device_id_from_cert(&cert_der), cert_der, key_der: key.serialize_der() })
    }

    /// Đọc `cert.der` + `key.der` trong `dir`, chưa có thì tạo mới.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let (cp, kp) = (dir.join("cert.der"), dir.join("key.der"));
        if cp.exists() && kp.exists() {
            let cert_der = std::fs::read(&cp)?;
            return Ok(Self { device_id: device_id_from_cert(&cert_der), cert_der, key_der: std::fs::read(&kp)? });
        }
        std::fs::create_dir_all(dir)?;
        let id = Self::generate()?;
        std::fs::write(&cp, &id.cert_der)?;
        std::fs::write(&kp, &id.key_der)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&kp, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(id)
    }
}
```

Đầu `pairing.rs`:
```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Mã 6 số giống nhau trên cả 2 máy nếu 2 máy thấy đúng chứng chỉ của nhau.
pub fn pairing_code(cert_a: &[u8], cert_b: &[u8]) -> String {
    let (x, y) = if cert_a <= cert_b { (cert_a, cert_b) } else { (cert_b, cert_a) };
    let mut h = Sha256::new();
    h.update((x.len() as u64).to_be_bytes());
    h.update(x);
    h.update(y);
    let d = h.finalize();
    let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000;
    format!("{n:06}")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub device_id: String,
    pub name: String,
    pub cert_fingerprint: String,
    pub last_addr: Option<String>,
}

pub struct PeerStore {
    path: PathBuf,
    peers: Vec<Peer>,
}

impl PeerStore {
    pub fn load(path: &Path) -> Result<Self> {
        let peers = if path.exists() { serde_json::from_slice(&std::fs::read(path)?)? } else { Vec::new() };
        Ok(Self { path: path.to_path_buf(), peers })
    }

    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // ghi file tạm rồi rename để không hỏng file khi mất điện giữa chừng
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.peers)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn list(&self) -> &[Peer] {
        &self.peers
    }

    pub fn add(&mut self, peer: Peer) {
        self.peers.retain(|p| p.device_id != peer.device_id);
        self.peers.push(peer);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.peers.len();
        self.peers.retain(|p| p.device_id != id);
        before != self.peers.len()
    }

    pub fn find_by_fingerprint(&self, fp: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.cert_fingerprint == fp)
    }

    pub fn set_addr(&mut self, id: &str, addr: &str) {
        if let Some(p) = self.peers.iter_mut().find(|p| p.device_id == id) {
            p.last_addr = Some(addr.to_string());
        }
    }
}
```

- [x] **Step 4: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core identity && cargo test -p lansync-core pairing`
Expected: `3 passed` và `3 passed`.

- [x] **Step 5: Commit**

```bash
git add crates/lansync-core
git commit -m "feat(core): danh tính máy, mã ghép cặp và peers.json"
```

---

### Task 6: Kết nối TLS 1.3 hai chiều (`transport.rs`)

**Files:**
- Create: `crates/lansync-core/src/transport.rs`
- Modify: `crates/lansync-core/src/lib.rs` (thêm `pub mod transport;`)

**Interfaces:**
- Consumes: `identity::Identity`
- Produces:
  - `transport::server_config(&Identity) -> Result<Arc<rustls::ServerConfig>>`
  - `transport::client_config(&Identity) -> Result<Arc<rustls::ClientConfig>>`
  - `transport::SecureConn { stream: tokio_rustls::TlsStream<TcpStream>, peer_cert: Vec<u8>, peer_addr: SocketAddr }`
  - `transport::connect(addr: SocketAddr, cfg: Arc<ClientConfig>) -> Result<SecureConn>`
  - `transport::accept(tcp: TcpStream, cfg: Arc<ServerConfig>) -> Result<SecureConn>`

Cách xác thực: lớp TLS chấp nhận mọi chứng chỉ nhưng **vẫn kiểm tra chữ ký handshake**. Việc chứng chỉ có thuộc máy đã ghép cặp hay không do `engine` quyết định, bằng cách so `fingerprint(peer_cert)` với `peers.json`.

- [x] **Step 1: Viết test (failing)**

`crates/lansync-core/src/transport.rs`:
```rust
//! TCP + TLS 1.3 xác thực hai chiều bằng chứng chỉ tự ký.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::protocol::{read_msg, write_msg, Message};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn mutual_tls_exposes_both_certs() {
        let (a, b) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let scfg = server_config(&b).unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut c = accept(tcp, scfg).await.unwrap();
            let m = read_msg(&mut c.stream).await.unwrap();
            (c.peer_cert, m)
        });
        let mut client = connect(addr, client_config(&a).unwrap()).await.unwrap();
        write_msg(&mut client.stream, &Message::Ping).await.unwrap();
        let (server_saw, msg) = server.await.unwrap();
        assert_eq!(client.peer_cert, b.cert_der);
        assert_eq!(server_saw, a.cert_der);
        assert_eq!(msg, Message::Ping);
    }
}
```

- [x] **Step 2: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core transport`
Expected: lỗi biên dịch.

- [x] **Step 3: Implement**

Đầu `transport.rs`:
```rust
use crate::identity::Identity;
use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

pub struct SecureConn {
    pub stream: TlsStream<TcpStream>,
    /// chứng chỉ DER của máy bên kia
    pub peer_cert: Vec<u8>,
    pub peer_addr: SocketAddr,
}

/// Chấp nhận mọi chứng chỉ nhưng vẫn kiểm tra chữ ký handshake.
/// Engine sẽ so fingerprint với danh sách máy đã ghép cặp.
#[derive(Debug)]
struct AcceptAnyCert {
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime)
        -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

impl ClientCertVerifier for AcceptAnyCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime)
        -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

fn own_cert(id: &Identity) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    (
        vec![CertificateDer::from(id.cert_der.clone())],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key_der.clone())),
    )
}

pub fn server_config(id: &Identity) -> Result<Arc<ServerConfig>> {
    let p = provider();
    let verifier = Arc::new(AcceptAnyCert { algs: p.signature_verification_algorithms });
    let (certs, key) = own_cert(id);
    let cfg = ServerConfig::builder_with_provider(p)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)?;
    Ok(Arc::new(cfg))
}

pub fn client_config(id: &Identity) -> Result<Arc<ClientConfig>> {
    let p = provider();
    let verifier = Arc::new(AcceptAnyCert { algs: p.signature_verification_algorithms });
    let (certs, key) = own_cert(id);
    let cfg = ClientConfig::builder_with_provider(p)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(certs, key)?;
    Ok(Arc::new(cfg))
}

pub async fn connect(addr: SocketAddr, cfg: Arc<ClientConfig>) -> Result<SecureConn> {
    let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr))
        .await
        .context("hết thời gian kết nối")??;
    tcp.set_nodelay(true)?;
    let name = ServerName::try_from("lansync.local")?.to_owned();
    let tls = TlsConnector::from(cfg).connect(name, tcp).await?;
    let peer_cert = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).map(|c| c.to_vec())
        .context("máy kia không gửi chứng chỉ")?;
    Ok(SecureConn { stream: tls.into(), peer_cert, peer_addr: addr })
}

pub async fn accept(tcp: TcpStream, cfg: Arc<ServerConfig>) -> Result<SecureConn> {
    tcp.set_nodelay(true)?;
    let peer_addr = tcp.peer_addr()?;
    let tls = TlsAcceptor::from(cfg).accept(tcp).await?;
    let peer_cert = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).map(|c| c.to_vec())
        .context("máy kia không gửi chứng chỉ")?;
    Ok(SecureConn { stream: tls.into(), peer_cert, peer_addr })
}
```

- [x] **Step 4: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core transport`
Expected: `1 passed`.

- [x] **Step 5: Commit**

```bash
git add crates/lansync-core
git commit -m "feat(core): kết nối TLS 1.3 xác thực hai chiều"
```

---

### Task 7: Gửi/nhận file (`transfer.rs`)

**Files:**
- Create: `crates/lansync-core/src/transfer.rs`
- Modify: `crates/lansync-core/src/lib.rs` (thêm `pub mod transfer;`)

**Interfaces:**
- Consumes: `protocol::{write_msg, Message, Entry, Hash, CHUNK_SIZE}`, `paths::to_fs_path`, `fsutil::{hash_file, mtime_ns, filetime_from_ns}`
- Produces:
  - `transfer::send_file<W: AsyncWrite + Unpin>(w: &mut W, root: &Path, path: &str, hash: &Hash) -> Result<bool>`: `false` nếu không mở được file (chưa ghi gì vào stream)
  - `transfer::Incoming::begin(root: &Path, dest_rel: &str, size: u64, hash: Hash, mtime_ns: i64) -> Incoming` (async, không lỗi; lỗi được ghi lại bên trong)
  - `Incoming::chunk(&mut self, data: &[u8])` (async)
  - `Incoming::finish(self, expected_local: Option<Hash>) -> Result<Entry, FinishError>` (async)
  - `transfer::FinishError::{HashMismatch, LocalChanged, Io(String)}`

- [x] **Step 1: Viết test (failing)**

`crates/lansync-core/src/transfer.rs`:
```rust
//! Gửi file theo chunk; nhận vào file tạm, kiểm tra hash, rồi rename.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::read_msg;

    fn tmp_files(dir: &Path) -> Vec<String> {
        walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".lansync-tmp-")).collect()
    }

    #[tokio::test]
    async fn sends_and_receives_file() {
        let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::create_dir_all(src.path().join("sub")).unwrap();
        let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
        let sp = src.path().join("sub/x.bin");
        std::fs::write(&sp, &data).unwrap();
        let hash = hash_file(&sp).unwrap();

        let (mut a, mut b) = tokio::io::duplex(1 << 20);
        let root = src.path().to_path_buf();
        let sender = tokio::spawn(async move { send_file(&mut a, &root, "sub/x.bin", &hash).await.unwrap() });

        let Message::FileHeader { size, hash: h, mtime_ns: m, .. } = read_msg(&mut b).await.unwrap() else { panic!("cần FileHeader") };
        let mut inc = Incoming::begin(dst.path(), "sub/x.bin", size, h, m).await;
        loop {
            match read_msg(&mut b).await.unwrap() {
                Message::FileChunk { data } => inc.chunk(&data).await,
                Message::FileEnd => break,
                other => panic!("message lạ {other:?}"),
            }
        }
        assert!(sender.await.unwrap());
        let entry = inc.finish(None).await.unwrap();

        assert_eq!(std::fs::read(dst.path().join("sub/x.bin")).unwrap(), data);
        assert_eq!(entry.hash, Some(hash));
        let src_m = mtime_ns(&std::fs::metadata(&sp).unwrap());
        assert!((entry.mtime_ns - src_m).abs() < 1_000, "mtime phải theo máy gửi");
        assert!(tmp_files(dst.path()).is_empty());
    }

    #[tokio::test]
    async fn hash_mismatch_is_rejected_and_tmp_removed() {
        let dst = tempfile::tempdir().unwrap();
        let mut inc = Incoming::begin(dst.path(), "a.txt", 3, [0; 32], 0).await;
        inc.chunk(b"abc").await;
        assert!(matches!(inc.finish(None).await, Err(FinishError::HashMismatch)));
        assert!(!dst.path().join("a.txt").exists());
        assert!(tmp_files(dst.path()).is_empty());
    }

    #[tokio::test]
    async fn refuses_to_overwrite_locally_changed_file() {
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(dst.path().join("a.txt"), b"ban moi cua toi").unwrap();
        let new = *blake3::hash(b"tu may kia").as_bytes();
        let mut inc = Incoming::begin(dst.path(), "a.txt", 10, new, 0).await;
        inc.chunk(b"tu may kia").await;
        let old = *blake3::hash(b"ban cu").as_bytes();
        assert!(matches!(inc.finish(Some(old)).await, Err(FinishError::LocalChanged)));
        assert_eq!(std::fs::read(dst.path().join("a.txt")).unwrap(), b"ban moi cua toi");
    }

    #[tokio::test]
    async fn send_file_returns_false_when_missing() {
        let src = tempfile::tempdir().unwrap();
        let (mut a, _b) = tokio::io::duplex(1024);
        assert!(!send_file(&mut a, src.path(), "khong-co.txt", &[0; 32]).await.unwrap());
    }
}
```

- [x] **Step 2: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core transfer`
Expected: lỗi biên dịch.

- [x] **Step 3: Implement**

Đầu `transfer.rs`:
```rust
use crate::fsutil::{filetime_from_ns, hash_file, mtime_ns};
use crate::paths::to_fs_path;
use crate::protocol::{write_msg, Entry, Hash, Message, CHUNK_SIZE};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Gửi FileHeader + các FileChunk + FileEnd. Trả `false` nếu không mở được file
/// (khi đó chưa ghi gì vào stream, engine sẽ gửi FileUnavailable).
pub async fn send_file<W: AsyncWrite + Unpin>(w: &mut W, root: &Path, path: &str, hash: &Hash) -> Result<bool> {
    let Ok(mut f) = tokio::fs::File::open(to_fs_path(root, path)).await else { return Ok(false) };
    let meta = f.metadata().await?;
    write_msg(w, &Message::FileHeader { path: path.to_string(), size: meta.len(), hash: *hash, mtime_ns: mtime_ns(&meta) }).await?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        write_msg(w, &Message::FileChunk { data: buf[..n].to_vec() }).await?;
    }
    write_msg(w, &Message::FileEnd).await?;
    Ok(true)
}

#[derive(Debug, thiserror::Error)]
pub enum FinishError {
    #[error("dữ liệu nhận được không khớp hash")]
    HashMismatch,
    #[error("file ở máy này vừa bị thay đổi")]
    LocalChanged,
    #[error("lỗi ghi file: {0}")]
    Io(String),
}

fn io_err(e: impl std::fmt::Display) -> FinishError {
    FinishError::Io(e.to_string())
}

/// File đang nhận. Ghi vào `.lansync-tmp-*` cùng thư mục đích. Nếu bị drop trước khi
/// `finish` thành công thì file tạm bị xoá.
pub struct Incoming {
    root: PathBuf,
    dest_rel: String,
    tmp: PathBuf,
    file: Option<tokio::fs::File>,
    hasher: blake3::Hasher,
    size: u64,
    received: u64,
    hash: Hash,
    mtime_ns: i64,
    error: Option<String>,
    done: bool,
}

impl Incoming {
    pub async fn begin(root: &Path, dest_rel: &str, size: u64, hash: Hash, mtime_ns: i64) -> Self {
        let dest = to_fs_path(root, dest_rel);
        let dir = dest.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
        let tmp = dir.join(format!(".lansync-tmp-{:016x}", rand::random::<u64>()));
        let mut me = Self {
            root: root.to_path_buf(), dest_rel: dest_rel.to_string(), tmp: tmp.clone(), file: None,
            hasher: blake3::Hasher::new(), size, received: 0, hash, mtime_ns, error: None, done: false,
        };
        let opened = async {
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::File::create(&tmp).await
        }
        .await;
        match opened {
            Ok(f) => me.file = Some(f),
            Err(e) => me.error = Some(e.to_string()),
        }
        me
    }

    /// Ghi một chunk. Lỗi (ví dụ ổ đầy) được ghi lại và báo ở `finish`.
    pub async fn chunk(&mut self, data: &[u8]) {
        if self.error.is_some() {
            return;
        }
        self.received += data.len() as u64;
        self.hasher.update(data);
        if let Some(f) = self.file.as_mut() {
            if let Err(e) = f.write_all(data).await {
                self.error = Some(e.to_string());
            }
        }
    }

    pub async fn finish(mut self, expected_local: Option<Hash>) -> Result<Entry, FinishError> {
        if let Some(e) = self.error.take() {
            return Err(FinishError::Io(e));
        }
        let mut f = self.file.take().ok_or_else(|| io_err("file tạm chưa mở"))?;
        f.flush().await.map_err(io_err)?;
        f.sync_all().await.map_err(io_err)?;
        drop(f);
        if self.received != self.size || *self.hasher.finalize().as_bytes() != self.hash {
            return Err(FinishError::HashMismatch);
        }

        // không ghi đè nếu người dùng vừa sửa file đích sau lúc lập kế hoạch
        let dest = to_fs_path(&self.root, &self.dest_rel);
        let d = dest.clone();
        let current = tokio::task::spawn_blocking(move || if d.is_file() { hash_file(&d).map(Some) } else { Ok(None) })
            .await
            .map_err(io_err)?
            .map_err(io_err)?;
        if current != expected_local {
            return Err(FinishError::LocalChanged);
        }

        filetime::set_file_mtime(&self.tmp, filetime_from_ns(self.mtime_ns)).map_err(io_err)?;
        // Windows: file đang mở bởi app khác (Office) → thử lại sau 10 giây, tối đa 30 lần
        let mut attempts = 0;
        loop {
            match tokio::fs::rename(&self.tmp, &dest).await {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && attempts < 30 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
                Err(e) => return Err(io_err(e)),
            }
        }
        self.done = true;
        let meta = std::fs::metadata(&dest).map_err(io_err)?;
        Ok(Entry { path: self.dest_rel.clone(), size: meta.len(), mtime_ns: mtime_ns(&meta), hash: Some(self.hash) })
    }
}

impl Drop for Incoming {
    fn drop(&mut self) {
        if !self.done {
            drop(self.file.take());
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}
```

- [x] **Step 4: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core transfer`
Expected: `4 passed`.

- [x] **Step 5: Commit**

```bash
git add crates/lansync-core
git commit -m "feat(core): gửi/nhận file qua file tạm có kiểm tra hash"
```

---

### Task 8: Theo dõi thư mục + tìm máy mDNS (`watcher.rs`, `discovery.rs`)

**Files:**
- Create: `crates/lansync-core/src/watcher.rs`, `crates/lansync-core/src/discovery.rs`
- Modify: `crates/lansync-core/src/lib.rs` (thêm `pub mod watcher; pub mod discovery;`)

**Interfaces:**
- Produces:
  - `watcher::spawn(root: PathBuf, tx: tokio::sync::mpsc::UnboundedSender<()>) -> Result<WatchGuard>`: gửi `()` sau mỗi đợt thay đổi (debounce 1 giây). Drop `WatchGuard` thì dừng theo dõi.
  - `discovery::DiscoveredPeer { device_id: String, name: String, addrs: Vec<SocketAddr> }` (Clone, Debug)
  - `discovery::Discovery::start(device_id: &str, name: &str, port: u16) -> Result<Discovery>`
  - `Discovery::peers(&self) -> Vec<DiscoveredPeer>` (không gồm chính mình, IPv4 xếp trước)
  - `Discovery::shutdown(&self)`

- [x] **Step 1: Viết test (failing)**

`crates/lansync-core/src/watcher.rs`:
```rust
//! Theo dõi thay đổi trong thư mục đồng bộ (debounce 1 giây).

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn notifies_on_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _guard = spawn(dir.path().to_path_buf(), tx).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(matches!(got, Ok(Some(()))), "phải nhận được thông báo thay đổi");
    }
}
```

`crates/lansync-core/src/discovery.rs`:
```rust
//! Quảng bá và tìm máy LanSync trong mạng LAN bằng mDNS.

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    // Cần mạng multicast thật; chạy tay: cargo test -p lansync-core discovery -- --ignored
    #[test]
    #[ignore]
    fn two_instances_see_each_other() {
        let a = Discovery::start("AAAAAAAAAAAAAAAA", "May A", 47901).unwrap();
        let b = Discovery::start("BBBBBBBBBBBBBBBB", "May B", 47902).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            let seen = a.peers().iter().any(|p| p.device_id == "BBBBBBBBBBBBBBBB" && p.addrs.iter().all(|s| s.port() == 47902));
            if seen && a.peers().iter().all(|p| p.device_id != "AAAAAAAAAAAAAAAA") {
                a.shutdown();
                b.shutdown();
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        panic!("không thấy nhau sau 10 giây: {:?}", a.peers());
    }
}
```

- [x] **Step 2: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core watcher`
Expected: lỗi biên dịch.

- [x] **Step 3: Implement**

Đầu `watcher.rs`. Debouncer chạy trong một thread riêng để không phải bận tâm nó có `Send` hay không:
```rust
use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// Drop guard → thread theo dõi kết thúc.
pub struct WatchGuard(#[allow(dead_code)] std::sync::mpsc::Sender<()>);

pub fn spawn(root: PathBuf, tx: UnboundedSender<()>) -> Result<WatchGuard> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new().name("lansync-watcher".into()).spawn(move || {
        let setup = (|| -> notify::Result<_> {
            let mut d = new_debouncer(Duration::from_secs(1), None, move |res: DebounceEventResult| {
                if res.is_ok() {
                    let _ = tx.send(());
                }
            })?;
            d.watch(&root, RecursiveMode::Recursive)?;
            Ok(d)
        })();
        match setup {
            Ok(d) => {
                let _ = ready_tx.send(Ok(()));
                let _ = stop_rx.recv(); // chờ tới khi WatchGuard bị drop
                drop(d);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
            }
        }
    })?;
    ready_rx.recv()?.map_err(anyhow::Error::msg)?;
    Ok(WatchGuard(stop_tx))
}
```
> Nếu `notify-debouncer-full` bản đang dùng chưa có `Debouncer::watch`, thay bằng `d.watcher().watch(&root, RecursiveMode::Recursive)?;`.

Đầu `discovery.rs`:
```rust
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

pub const SERVICE: &str = "_lansync._tcp.local.";

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub name: String,
    pub addrs: Vec<SocketAddr>,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    peers: Arc<Mutex<HashMap<String, DiscoveredPeer>>>,
}

impl Discovery {
    pub fn start(device_id: &str, name: &str, port: u16) -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let host = format!("{}.local.", device_id.to_lowercase());
        let props = [("id", device_id), ("name", name), ("v", "1")];
        let info = ServiceInfo::new(SERVICE, device_id, &host, "", port, &props[..])?.enable_addr_auto();
        daemon.register(info)?;

        let peers: Arc<Mutex<HashMap<String, DiscoveredPeer>>> = Arc::default();
        let rx = daemon.browse(SERVICE)?;
        let (me, map) = (device_id.to_string(), peers.clone());
        std::thread::Builder::new().name("lansync-mdns".into()).spawn(move || {
            while let Ok(ev) = rx.recv() {
                match ev {
                    ServiceEvent::ServiceResolved(info) => {
                        let Some(id) = info.get_property_val_str("id").map(str::to_string) else { continue };
                        if id == me {
                            continue;
                        }
                        let name = info.get_property_val_str("name").unwrap_or("?").to_string();
                        let mut addrs: Vec<SocketAddr> =
                            info.get_addresses().iter().map(|ip| SocketAddr::new(*ip, info.get_port())).collect();
                        addrs.sort_by_key(|a| !a.is_ipv4()); // IPv4 trước
                        map.lock().unwrap().insert(id.clone(), DiscoveredPeer { device_id: id, name, addrs });
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        if let Some(id) = fullname.split('.').next() {
                            map.lock().unwrap().remove(id);
                        }
                    }
                    _ => {}
                }
            }
        })?;
        Ok(Self { daemon, peers })
    }

    pub fn peers(&self) -> Vec<DiscoveredPeer> {
        self.peers.lock().unwrap().values().cloned().collect()
    }

    pub fn shutdown(&self) {
        let _ = self.daemon.shutdown();
    }
}
```
> `mdns-sd` thay đổi API khá thường xuyên. Nếu `get_addresses()` trả về kiểu khác `HashSet<IpAddr>` (ví dụ `ScopedIp` ở các bản mới), hãy đổi `*ip` thành `ip.to_ip_addr()` và giữ nguyên các phần còn lại.

- [x] **Step 4: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core watcher`
Expected: `1 passed`.
Run: `cargo test -p lansync-core discovery -- --ignored`
Expected: `1 passed` (cần Wi-Fi/LAN; nếu macOS hỏi quyền Local Network thì bấm Allow).

- [x] **Step 5: Commit**

```bash
git add crates/lansync-core
git commit -m "feat(core): theo dõi thư mục và tìm máy bằng mDNS"
```

---

### Task 9: Config + Engine + test tích hợp ghép cặp/đồng bộ

**Files:**
- Create: `crates/lansync-core/src/config.rs`, `crates/lansync-core/src/engine.rs`, `crates/lansync-core/tests/sync.rs`
- Modify: `crates/lansync-core/src/lib.rs`

**Interfaces:**
- Consumes: toàn bộ module từ Task 1–8 với đúng chữ ký đã ghi ở trên.
- Produces:
  - `Config { sync_dir: PathBuf, data_dir: PathBuf, device_name: String, port: u16, enable_mdns: bool, rescan_interval: Duration }` + `Config::default_for_os() -> Result<Config>` + `Config::DEFAULT_PORT = 47800`
  - `Engine::start(Config) -> Result<EngineHandle>` (async)
  - `EngineHandle` (Clone): `subscribe() -> broadcast::Receiver<EngineEvent>`, `device_id() -> &str`, `device_name() -> &str`, `local_port() -> u16`, `sync_dir() -> &Path`, `paired_peers() -> Vec<Peer>`, `online_peers() -> Vec<String>`, `discovered_peers() -> Vec<DiscoveredPeer>`, `async pair(SocketAddr) -> Result<()>`, `confirm_pairing(bool)`, `connect_peer(SocketAddr)`, `unpair(&str) -> Result<()>`, `async shutdown(&self)`
  - `EngineEvent::{Status(SyncStatus), PeerOnline { id, name }, PeerOffline { id }, PairingRequest { peer_name, code }, PairingResult { peer_id, ok }, FileSynced { path }, ConflictCreated { path }, Skipped { path, reason }, Error(String)}`
  - `SyncStatus::{Idle, Syncing { pending: usize }, Offline}`

Cách engine chạy (để người đọc code dễ hình dung):
```
accept_loop ─┐                      ┌─ writer_loop  (gửi Out::Msg / Out::File lần lượt)
dial_loop  ──┼─► handle_conn ─Hello─┼─ reader_loop  (đọc frame, chuyển cho puller/writer/download đang chờ)
dial_worker ─┘     │                ├─ puller_loop  (lập kế hoạch → tải từng file một)
                   └─ pair_session  └─ ping_loop    (Ping mỗi 15 giây)
scan_loop (watcher + quét lại mỗi rescan_interval) ─► broadcast IndexUpdate cho mọi phiên
```

- [x] **Step 1: Khai báo module + re-export**

`lib.rs` (bản đầy đủ):
```rust
//! LanSync core: đồng bộ thư mục 2 chiều qua mạng LAN.
pub mod config;
pub mod discovery;
pub mod engine;
pub mod fsutil;
pub mod identity;
pub mod index;
pub mod pairing;
pub mod paths;
pub mod planner;
pub mod protocol;
pub mod transfer;
pub mod transport;
pub mod watcher;

pub use config::Config;
pub use discovery::DiscoveredPeer;
pub use engine::{Engine, EngineEvent, EngineHandle, SyncStatus};
pub use pairing::Peer;
```

- [x] **Step 2: Viết test tích hợp (failing)**

`crates/lansync-core/tests/sync.rs`:
```rust
//! Test tích hợp: 2 engine trong cùng process, kết nối qua 127.0.0.1, tắt mDNS.
use lansync_core::{Config, Engine, EngineEvent, EngineHandle};
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

pub fn cfg(root: &Path, name: &str) -> Config {
    Config {
        sync_dir: root.join(name).join("sync"),
        data_dir: root.join(name).join("data"),
        device_name: name.into(),
        port: 0,
        enable_mdns: false,
        rescan_interval: Duration::from_secs(2),
    }
}

pub fn addr(h: &EngineHandle) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], h.local_port()))
}

pub async fn wait_until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    f()
}

pub fn read(p: &Path) -> Option<Vec<u8>> {
    std::fs::read(p).ok()
}

/// Ghép cặp a ↔ b, tự bấm "Xác nhận" ở cả hai bên, kiểm tra hai mã giống nhau.
pub async fn pair(a: &EngineHandle, b: &EngineHandle) {
    let mut codes = Vec::new();
    for h in [a, b] {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let (h2, mut events) = (h.clone(), h.subscribe());
        tokio::spawn(async move {
            while let Ok(ev) = events.recv().await {
                if let EngineEvent::PairingRequest { code, .. } = ev {
                    h2.confirm_pairing(true);
                    let _ = tx.send(code);
                    break;
                }
            }
        });
        codes.push(rx);
    }
    a.pair(addr(b)).await.unwrap();
    let (ca, cb) = (codes.remove(0).await.unwrap(), codes.remove(0).await.unwrap());
    assert_eq!(ca, cb, "hai máy phải hiện cùng một mã");
    assert!(wait_until(10, || a.paired_peers().len() == 1 && b.paired_peers().len() == 1).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairs_and_syncs_new_and_modified_files_both_ways() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Engine::start(cfg(tmp.path(), "mac")).await.unwrap();
    let b = Engine::start(cfg(tmp.path(), "win")).await.unwrap();
    std::fs::write(a.sync_dir().join("hello.txt"), b"xin chao").unwrap();

    pair(&a, &b).await;

    let bp = b.sync_dir().join("hello.txt");
    assert!(wait_until(15, || read(&bp).as_deref() == Some(b"xin chao")).await, "Mac → Win");

    std::fs::create_dir_all(b.sync_dir().join("sub")).unwrap();
    std::fs::write(b.sync_dir().join("sub").join("tu-win.txt"), b"tu windows").unwrap();
    let ap = a.sync_dir().join("sub").join("tu-win.txt");
    assert!(wait_until(15, || read(&ap).as_deref() == Some(b"tu windows")).await, "Win → Mac");

    std::fs::write(&bp, b"phien ban 2 dai hon").unwrap();
    let ah = a.sync_dir().join("hello.txt");
    assert!(wait_until(15, || read(&ah).as_deref() == Some(b"phien ban 2 dai hon")).await, "sửa file");

    a.shutdown().await;
    b.shutdown().await;
}
```

- [x] **Step 3: Chạy test, xác nhận FAIL**

Run: `cargo test -p lansync-core --test sync`
Expected: lỗi biên dịch (`config`, `engine` chưa có).

- [x] **Step 4: Implement `config.rs`**

```rust
//! Cấu hình engine.
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    /// Thư mục đồng bộ (~/LanSync hoặc %USERPROFILE%\LanSync)
    pub sync_dir: PathBuf,
    /// Dữ liệu app: index.db, cert.der, key.der, peers.json
    pub data_dir: PathBuf,
    pub device_name: String,
    /// 0 = cổng ngẫu nhiên. Cổng mặc định bị chiếm thì tự lùi về cổng ngẫu nhiên.
    pub port: u16,
    pub enable_mdns: bool,
    pub rescan_interval: Duration,
}

impl Config {
    pub const DEFAULT_PORT: u16 = 47800;

    pub fn default_for_os() -> Result<Self> {
        let user = directories::UserDirs::new().context("không tìm thấy thư mục người dùng")?;
        let base = directories::BaseDirs::new().context("không tìm thấy thư mục dữ liệu app")?;
        let device_name = hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "LanSync".into());
        Ok(Self {
            sync_dir: user.home_dir().join("LanSync"),
            // macOS: ~/Library/Application Support/LanSync — Windows: %APPDATA%\LanSync
            data_dir: base.data_dir().join("LanSync"),
            device_name,
            port: Self::DEFAULT_PORT,
            enable_mdns: true,
            rescan_interval: Duration::from_secs(300),
        })
    }
}
```

- [x] **Step 5: Implement `engine.rs`**

```rust
//! Engine: nối index, watcher, discovery, transport, planner, transfer lại với nhau.
use crate::config::Config;
use crate::discovery::{DiscoveredPeer, Discovery};
use crate::identity::{device_id_from_cert, fingerprint, Identity};
use crate::index::Index;
use crate::pairing::{pairing_code, Peer, PeerStore};
use crate::paths::to_fs_path;
use crate::planner::{plan, Action, PlanInput};
use crate::protocol::{index_messages, read_msg, write_msg, Entry, Hash, Message, PROTO_VERSION};
use crate::transfer::{send_file, FinishError, Incoming};
use crate::transport::{self, SecureConn};
use crate::watcher;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_rustls::TlsStream;
use tokio_util::sync::CancellationToken;

type Stream = TlsStream<TcpStream>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    Idle,
    Syncing { pending: usize },
    Offline,
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    Status(SyncStatus),
    PeerOnline { id: String, name: String },
    PeerOffline { id: String },
    PairingRequest { peer_name: String, code: String },
    PairingResult { peer_id: String, ok: bool },
    FileSynced { path: String },
    ConflictCreated { path: String },
    Skipped { path: String, reason: String },
    Error(String),
}

/// Việc writer cần làm. File được gửi trọn vẹn (header + chunks + end) trước khi gửi message khác.
enum Out {
    Msg(Message),
    File { path: String, hash: Hash },
}

enum Trigger {
    RemoteFull(Vec<Entry>),
    RemoteUpdate(Vec<Entry>),
    Tick,
}

enum DownloadResult {
    Received(Incoming),
    Unavailable,
}

struct PendingDownload {
    remote_path: String,
    local_path: String,
    tx: oneshot::Sender<DownloadResult>,
}

type Pending = Arc<Mutex<Option<PendingDownload>>>;

struct SessionSlot {
    id: u64,
    initiator: String,
    outbox: mpsc::UnboundedSender<Out>,
    trigger: mpsc::UnboundedSender<Trigger>,
    cancel: CancellationToken,
}

struct Inner {
    cfg: Config,
    identity: Identity,
    index: Arc<Mutex<Index>>,
    peers: Mutex<PeerStore>,
    sessions: Mutex<HashMap<String, SessionSlot>>,
    pending_pair: Mutex<Option<oneshot::Sender<bool>>>,
    skipped_reported: Mutex<HashSet<String>>,
    events: broadcast::Sender<EngineEvent>,
    discovery: Option<Discovery>,
    local_port: u16,
    server_cfg: Arc<rustls::ServerConfig>,
    client_cfg: Arc<rustls::ClientConfig>,
    dial_tx: mpsc::UnboundedSender<SocketAddr>,
    next_session_id: AtomicU64,
}

pub struct Engine;

#[derive(Clone)]
pub struct EngineHandle {
    inner: Arc<Inner>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Engine {
    pub async fn start(cfg: Config) -> Result<EngineHandle> {
        std::fs::create_dir_all(&cfg.sync_dir)?;
        std::fs::create_dir_all(&cfg.data_dir)?;
        let identity = Identity::load_or_create(&cfg.data_dir)?;
        let (db, root) = (cfg.data_dir.join("index.db"), cfg.sync_dir.clone());
        let (index, first_scan) = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut idx = Index::open(&db, &root)?;
            let report = idx.scan()?;
            Ok((idx, report))
        })
        .await??;
        let peers = PeerStore::load(&cfg.data_dir.join("peers.json"))?;

        let listener = match TcpListener::bind(("0.0.0.0", cfg.port)).await {
            Ok(l) => l,
            Err(e) if cfg.port != 0 => {
                tracing::warn!("cổng {} bận ({e}), dùng cổng ngẫu nhiên", cfg.port);
                TcpListener::bind(("0.0.0.0", 0)).await?
            }
            Err(e) => return Err(e.into()),
        };
        let local_port = listener.local_addr()?.port();
        let discovery = if cfg.enable_mdns {
            Discovery::start(&identity.device_id, &cfg.device_name, local_port)
                .map_err(|e| tracing::warn!("mDNS lỗi, chỉ kết nối được bằng IP: {e:#}"))
                .ok()
        } else {
            None
        };

        let (events, _) = broadcast::channel(512);
        let (dial_tx, dial_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            server_cfg: transport::server_config(&identity)?,
            client_cfg: transport::client_config(&identity)?,
            cfg,
            identity,
            index: Arc::new(Mutex::new(index)),
            peers: Mutex::new(peers),
            sessions: Mutex::default(),
            pending_pair: Mutex::default(),
            skipped_reported: Mutex::default(),
            events,
            discovery,
            local_port,
            dial_tx,
            next_session_id: AtomicU64::new(1),
        });
        inner.report_skipped(first_scan.skipped);

        let tasks = vec![
            tokio::spawn(accept_loop(inner.clone(), listener)),
            tokio::spawn(scan_loop(inner.clone())),
            tokio::spawn(dial_loop(inner.clone())),
            tokio::spawn(dial_worker(inner.clone(), dial_rx)),
        ];
        inner.emit(EngineEvent::Status(SyncStatus::Offline));
        Ok(EngineHandle { inner, tasks: Arc::new(Mutex::new(tasks)) })
    }
}

impl EngineHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }
    pub fn device_id(&self) -> &str {
        &self.inner.identity.device_id
    }
    pub fn device_name(&self) -> &str {
        &self.inner.cfg.device_name
    }
    pub fn local_port(&self) -> u16 {
        self.inner.local_port
    }
    pub fn sync_dir(&self) -> &Path {
        &self.inner.cfg.sync_dir
    }
    pub fn paired_peers(&self) -> Vec<Peer> {
        self.inner.peers.lock().unwrap().list().to_vec()
    }
    pub fn online_peers(&self) -> Vec<String> {
        self.inner.sessions.lock().unwrap().keys().cloned().collect()
    }
    pub fn discovered_peers(&self) -> Vec<DiscoveredPeer> {
        self.inner.discovery.as_ref().map(|d| d.peers()).unwrap_or_default()
    }
    /// Bắt đầu ghép cặp với máy ở `addr`. Trả về khi ghép cặp kết thúc (thành công hoặc không).
    pub async fn pair(&self, addr: SocketAddr) -> Result<()> {
        let conn = transport::connect(addr, self.inner.client_cfg.clone()).await?;
        handle_conn(self.inner.clone(), conn, true, true).await
    }
    /// Người dùng bấm Xác nhận (true) hoặc Huỷ (false) cho yêu cầu ghép cặp đang chờ.
    pub fn confirm_pairing(&self, accept: bool) {
        if let Some(tx) = self.inner.pending_pair.lock().unwrap().take() {
            let _ = tx.send(accept);
        }
    }
    /// Kết nối tới máy đã ghép cặp ở địa chỉ cụ thể (dùng khi mDNS không hoạt động).
    pub fn connect_peer(&self, addr: SocketAddr) {
        let _ = self.inner.dial_tx.send(addr);
    }
    pub fn unpair(&self, id: &str) -> Result<()> {
        {
            let mut ps = self.inner.peers.lock().unwrap();
            ps.remove(id);
            ps.save()?;
        }
        if let Some(slot) = self.inner.sessions.lock().unwrap().get(id) {
            slot.cancel.cancel();
        }
        Ok(())
    }
    pub async fn shutdown(&self) {
        for t in self.tasks.lock().unwrap().drain(..) {
            t.abort();
        }
        for slot in self.inner.sessions.lock().unwrap().values() {
            slot.cancel.cancel();
        }
        if let Some(d) = &self.inner.discovery {
            d.shutdown();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

impl Inner {
    fn emit(&self, e: EngineEvent) {
        let _ = self.events.send(e);
    }

    fn emit_status(&self) {
        let s = if self.sessions.lock().unwrap().is_empty() { SyncStatus::Offline } else { SyncStatus::Idle };
        self.emit(EngineEvent::Status(s));
    }

    fn report_skipped(&self, skipped: Vec<(String, String)>) {
        let mut seen = self.skipped_reported.lock().unwrap();
        for (path, reason) in skipped {
            if seen.insert(path.clone()) {
                tracing::warn!("bỏ qua {path}: {reason}");
                self.emit(EngineEvent::Skipped { path, reason });
            }
        }
    }

    /// Đăng ký phiên. Nếu đã có phiên với máy này: giữ phiên do máy có device_id nhỏ hơn khởi tạo.
    fn register_session(&self, peer_id: &str, initiator: &str, outbox: mpsc::UnboundedSender<Out>,
                        trigger: mpsc::UnboundedSender<Trigger>, cancel: CancellationToken) -> Option<u64> {
        let mut s = self.sessions.lock().unwrap();
        let preferred = std::cmp::min(self.identity.device_id.as_str(), peer_id).to_string();
        if let Some(existing) = s.get(peer_id) {
            if existing.initiator == preferred || initiator != preferred {
                return None;
            }
            existing.cancel.cancel();
        }
        let id = self.next_session_id.fetch_add(1, Ordering::SeqCst);
        s.insert(peer_id.to_string(), SessionSlot { id, initiator: initiator.to_string(), outbox, trigger, cancel });
        Some(id)
    }

    fn unregister_session(&self, peer_id: &str, id: u64) {
        let mut s = self.sessions.lock().unwrap();
        if s.get(peer_id).map(|x| x.id) == Some(id) {
            s.remove(peer_id);
        }
    }

    /// Báo cho mọi máy đang kết nối biết index ở máy này vừa đổi.
    fn broadcast_local_changes(&self, entries: Vec<Entry>) {
        if entries.is_empty() {
            return;
        }
        let msgs = index_messages(entries, false);
        for slot in self.sessions.lock().unwrap().values() {
            for m in &msgs {
                let _ = slot.outbox.send(Out::Msg(m.clone()));
            }
            let _ = slot.trigger.send(Trigger::Tick);
        }
    }

    async fn rescan(self: &Arc<Self>) -> Result<Vec<Entry>> {
        let me = self.clone();
        let report = tokio::task::spawn_blocking(move || {
            // thư mục bị xoá → tạo lại (file đã có trong index sẽ được đánh dấu đã xoá, không tải lại)
            std::fs::create_dir_all(&me.cfg.sync_dir)?;
            me.index.lock().unwrap().scan()
        })
        .await??;
        self.report_skipped(report.skipped);
        self.broadcast_local_changes(report.changed.clone());
        Ok(report.changed)
    }

    async fn dial(self: &Arc<Self>, addr: SocketAddr) -> Result<()> {
        let conn = transport::connect(addr, self.client_cfg.clone()).await?;
        handle_conn(self.clone(), conn, true, false).await
    }
}

async fn accept_loop(inner: Arc<Inner>, listener: TcpListener) {
    loop {
        let (tcp, addr) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("accept lỗi: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let inner = inner.clone();
        tokio::spawn(async move {
            let res = async {
                let conn = tokio::time::timeout(Duration::from_secs(10), transport::accept(tcp, inner.server_cfg.clone()))
                    .await
                    .context("hết thời gian bắt tay TLS")??;
                handle_conn(inner, conn, false, false).await
            }
            .await;
            if let Err(e) = res {
                tracing::info!("kết nối từ {addr} kết thúc: {e:#}");
            }
        });
    }
}

async fn dial_worker(inner: Arc<Inner>, mut rx: mpsc::UnboundedReceiver<SocketAddr>) {
    while let Some(addr) = rx.recv().await {
        let inner = inner.clone();
        tokio::spawn(async move {
            if let Err(e) = inner.dial(addr).await {
                tracing::debug!("kết nối tới {addr} kết thúc: {e:#}");
            }
        });
    }
}

/// Tự kết nối lại tới các máy đã ghép cặp, backoff 5 → 10 → 30 giây.
async fn dial_loop(inner: Arc<Inner>) {
    let mut backoff: HashMap<String, (Instant, Duration)> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let peers = inner.peers.lock().unwrap().list().to_vec();
        for peer in peers {
            if inner.sessions.lock().unwrap().contains_key(&peer.device_id) {
                backoff.remove(&peer.device_id);
                continue;
            }
            let now = Instant::now();
            if backoff.get(&peer.device_id).is_some_and(|(next, _)| now < *next) {
                continue;
            }
            let found = inner.discovery.as_ref()
                .and_then(|d| d.peers().into_iter().find(|p| p.device_id == peer.device_id))
                .and_then(|p| p.addrs.first().copied());
            let Some(addr) = found.or_else(|| peer.last_addr.as_deref().and_then(|a| a.parse().ok())) else { continue };
            let delay = match backoff.get(&peer.device_id).map(|(_, d)| d.as_secs()) {
                None => Duration::from_secs(5),
                Some(5) => Duration::from_secs(10),
                Some(_) => Duration::from_secs(30),
            };
            backoff.insert(peer.device_id.clone(), (now + delay, delay));
            let _ = inner.dial_tx.send(addr);
        }
    }
}

async fn scan_loop(inner: Arc<Inner>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _watch = watcher::spawn(inner.cfg.sync_dir.clone(), tx)
        .map_err(|e| tracing::warn!("không theo dõi được thư mục, chỉ quét định kỳ: {e:#}"))
        .ok();
    let mut tick = tokio::time::interval(inner.cfg.rescan_interval);
    tick.tick().await; // tick đầu tiên trả về ngay; đã quét lúc khởi động rồi
    loop {
        tokio::select! {
            Some(()) = rx.recv() => { while rx.try_recv().is_ok() {} }
            _ = tick.tick() => {}
        }
        if let Err(e) = inner.rescan().await {
            inner.emit(EngineEvent::Error(format!("Lỗi quét thư mục: {e:#}")));
        }
    }
}

/// Trao đổi Hello, rồi chuyển sang ghép cặp hoặc phiên đồng bộ.
async fn handle_conn(inner: Arc<Inner>, mut conn: SecureConn, dialed: bool, want_pair: bool) -> Result<()> {
    let hello = Message::Hello {
        device_id: inner.identity.device_id.clone(),
        name: inner.cfg.device_name.clone(),
        proto: PROTO_VERSION,
        pairing: want_pair,
        listen_port: inner.local_port,
    };
    write_msg(&mut conn.stream, &hello).await?;
    let reply = tokio::time::timeout(Duration::from_secs(10), read_msg(&mut conn.stream))
        .await
        .context("hết thời gian chờ Hello")??;
    let Message::Hello { name, proto, pairing, listen_port, .. } = reply else { bail!("máy kia không gửi Hello") };
    if proto != PROTO_VERSION {
        inner.emit(EngineEvent::Error(format!("Phiên bản LanSync trên {name} không tương thích")));
        bail!("proto {proto} không khớp");
    }
    // device_id tính từ chứng chỉ, không tin giá trị trong Hello
    let peer_id = device_id_from_cert(&conn.peer_cert);
    let fp = fingerprint(&conn.peer_cert);
    let peer_addr = SocketAddr::new(conn.peer_addr.ip(), listen_port);

    if pairing || want_pair {
        return pair_session(inner, conn, peer_id, name, fp, peer_addr, dialed).await;
    }
    let known = inner.peers.lock().unwrap().find_by_fingerprint(&fp).cloned();
    let Some(peer) = known else {
        tracing::warn!("từ chối máy chưa ghép cặp {name} ({peer_id})");
        bail!("máy chưa ghép cặp");
    };
    {
        let mut ps = inner.peers.lock().unwrap();
        ps.set_addr(&peer.device_id, &peer_addr.to_string());
        ps.save()?;
    }
    let initiator = if dialed { inner.identity.device_id.clone() } else { peer.device_id.clone() };
    run_session(inner, conn, peer, initiator).await
}

async fn pair_session(inner: Arc<Inner>, mut conn: SecureConn, peer_id: String, name: String, fp: String,
                      peer_addr: SocketAddr, dialed: bool) -> Result<()> {
    let code = pairing_code(&inner.identity.cert_der, &conn.peer_cert);
    let (tx, rx) = oneshot::channel();
    {
        let mut p = inner.pending_pair.lock().unwrap();
        if p.is_some() {
            bail!("đang có một yêu cầu ghép cặp khác");
        }
        *p = Some(tx);
    }
    inner.emit(EngineEvent::PairingRequest { peer_name: name.clone(), code });
    let local = tokio::time::timeout(Duration::from_secs(60), rx).await.ok().and_then(|r| r.ok()).unwrap_or(false);
    inner.pending_pair.lock().unwrap().take();
    // bên kia có thể đã đóng kết nối (đã huỷ) → lỗi ghi không quan trọng
    let _ = write_msg(&mut conn.stream, &Message::PairDecision { accept: local }).await;
    let remote = if local {
        matches!(
            tokio::time::timeout(Duration::from_secs(60), read_msg(&mut conn.stream)).await,
            Ok(Ok(Message::PairDecision { accept: true }))
        )
    } else {
        false
    };
    let ok = local && remote;
    if ok {
        let mut ps = inner.peers.lock().unwrap();
        ps.add(Peer { device_id: peer_id.clone(), name, cert_fingerprint: fp, last_addr: Some(peer_addr.to_string()) });
        ps.save()?;
    }
    inner.emit(EngineEvent::PairingResult { peer_id, ok });
    if ok && dialed {
        // máy khởi tạo ghép cặp kết nối lại ngay ở chế độ đồng bộ
        let _ = inner.dial_tx.send(peer_addr);
    }
    Ok(())
}

async fn run_session(inner: Arc<Inner>, conn: SecureConn, peer: Peer, initiator: String) -> Result<()> {
    let entries = inner.index.lock().unwrap().entries()?;
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Out>();
    let (trig_tx, trig_rx) = mpsc::unbounded_channel::<Trigger>();
    let cancel = CancellationToken::new();
    let Some(session_id) = inner.register_session(&peer.device_id, &initiator, out_tx.clone(), trig_tx.clone(), cancel.clone())
    else {
        tracing::debug!("bỏ kết nối trùng với {}", peer.name);
        return Ok(());
    };
    inner.emit(EngineEvent::PeerOnline { id: peer.device_id.clone(), name: peer.name.clone() });
    inner.emit_status();

    for m in index_messages(entries, true) {
        let _ = out_tx.send(Out::Msg(m));
    }
    let (rd, wr) = tokio::io::split(conn.stream);
    let pending: Pending = Arc::new(Mutex::new(None));
    let mut writer = tokio::spawn(writer_loop(inner.clone(), wr, out_rx));
    let puller = tokio::spawn(puller_loop(inner.clone(), peer.clone(), out_tx.clone(), pending.clone(), trig_rx));
    let pinger = tokio::spawn(ping_loop(out_tx.clone()));

    let result = tokio::select! {
        r = reader_loop(inner.clone(), rd, out_tx, pending, trig_tx) => r,
        r = &mut writer => r.context("writer bị huỷ").and_then(|r| r),
        _ = cancel.cancelled() => Ok(()),
    };
    writer.abort();
    puller.abort();
    pinger.abort();
    inner.unregister_session(&peer.device_id, session_id);
    inner.emit(EngineEvent::PeerOffline { id: peer.device_id.clone() });
    inner.emit_status();
    result
}

async fn ping_loop(out: mpsc::UnboundedSender<Out>) {
    let mut iv = tokio::time::interval(Duration::from_secs(15));
    iv.tick().await;
    loop {
        iv.tick().await;
        if out.send(Out::Msg(Message::Ping)).is_err() {
            break;
        }
    }
}

async fn writer_loop(inner: Arc<Inner>, mut w: WriteHalf<Stream>, mut rx: mpsc::UnboundedReceiver<Out>) -> Result<()> {
    while let Some(out) = rx.recv().await {
        match out {
            Out::Msg(m) => write_msg(&mut w, &m).await?,
            Out::File { path, hash } => {
                let current = inner.index.lock().unwrap().get(&path)?;
                let same = current.is_some_and(|e| e.hash == Some(hash));
                if !same || !send_file(&mut w, &inner.cfg.sync_dir, &path, &hash).await? {
                    write_msg(&mut w, &Message::FileUnavailable { path }).await?;
                }
            }
        }
    }
    Ok(())
}

async fn reader_loop(inner: Arc<Inner>, mut rd: ReadHalf<Stream>, out: mpsc::UnboundedSender<Out>, pending: Pending,
                     trig: mpsc::UnboundedSender<Trigger>) -> Result<()> {
    let mut incoming: Option<Incoming> = None;
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(45), read_msg(&mut rd))
            .await
            .context("máy kia không phản hồi quá 45 giây")??;
        match msg {
            Message::Index { entries } => {
                let _ = trig.send(Trigger::RemoteFull(entries));
            }
            Message::IndexUpdate { entries } => {
                let _ = trig.send(Trigger::RemoteUpdate(entries));
            }
            Message::GetFile { path, hash } => {
                let _ = out.send(Out::File { path, hash });
            }
            Message::FileHeader { path, size, hash, mtime_ns } => {
                let local_path = match pending.lock().unwrap().as_ref() {
                    Some(d) if d.remote_path == path => d.local_path.clone(),
                    _ => bail!("nhận FileHeader không mong đợi: {path}"),
                };
                incoming = Some(Incoming::begin(&inner.cfg.sync_dir, &local_path, size, hash, mtime_ns).await);
            }
            Message::FileChunk { data } => match incoming.as_mut() {
                Some(inc) => inc.chunk(&data).await,
                None => bail!("nhận FileChunk không mong đợi"),
            },
            Message::FileEnd => {
                let inc = incoming.take().context("nhận FileEnd không mong đợi")?;
                let waiting = pending.lock().unwrap().take();
                if let Some(d) = waiting {
                    let _ = d.tx.send(DownloadResult::Received(inc));
                }
            }
            Message::FileUnavailable { .. } => {
                let waiting = pending.lock().unwrap().take();
                if let Some(d) = waiting {
                    let _ = d.tx.send(DownloadResult::Unavailable);
                }
            }
            Message::Ping => {
                let _ = out.send(Out::Msg(Message::Pong));
            }
            Message::Pong => {}
            Message::Hello { .. } | Message::PairDecision { .. } => bail!("message không hợp lệ trong phiên đồng bộ"),
        }
    }
}

fn apply_trigger(t: Trigger, remote: &mut HashMap<String, Entry>, have_full: &mut bool) {
    match t {
        Trigger::RemoteFull(es) => {
            remote.clear();
            *have_full = true;
            remote.extend(es.into_iter().map(|e| (e.path.clone(), e)));
        }
        Trigger::RemoteUpdate(es) => remote.extend(es.into_iter().map(|e| (e.path.clone(), e))),
        Trigger::Tick => {}
    }
}

async fn puller_loop(inner: Arc<Inner>, peer: Peer, out: mpsc::UnboundedSender<Out>, pending: Pending,
                     mut trig: mpsc::UnboundedReceiver<Trigger>) {
    let mut remote: HashMap<String, Entry> = HashMap::new();
    let mut have_full = false;
    let mut given_up: HashSet<(String, Hash)> = HashSet::new();
    // vòng đồng bộ tự chạy lại mỗi 5 phút để thử lại các lỗi tạm thời (ổ đầy, file bị khoá...)
    let mut retry = tokio::time::interval(Duration::from_secs(300));
    retry.tick().await;
    loop {
        let first = tokio::select! {
            t = trig.recv() => match t { Some(t) => t, None => return },
            _ = retry.tick() => Trigger::Tick,
        };
        apply_trigger(first, &mut remote, &mut have_full);
        while let Ok(t) = trig.try_recv() {
            apply_trigger(t, &mut remote, &mut have_full);
        }
        if !have_full {
            continue;
        }
        if let Err(e) = sync_round(&inner, &peer, &out, &pending, &remote, &mut given_up).await {
            tracing::warn!("vòng đồng bộ với {} lỗi: {e:#}", peer.name);
            inner.emit(EngineEvent::Error(format!("Đồng bộ với {} lỗi: {e:#}", peer.name)));
        }
    }
}

async fn sync_round(inner: &Arc<Inner>, peer: &Peer, out: &mpsc::UnboundedSender<Out>, pending: &Pending,
                    remote: &HashMap<String, Entry>, given_up: &mut HashSet<(String, Hash)>) -> Result<()> {
    let remote_vec: Vec<Entry> = remote.values().cloned().collect();
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M").to_string();
    let actions = {
        let idx = inner.index.lock().unwrap();
        let local = idx.entries()?;
        let base = idx.bases(&peer.device_id)?;
        plan(&PlanInput {
            local: &local,
            remote: &remote_vec,
            base: &base,
            i_win_conflicts: inner.identity.device_id < peer.device_id,
            my_name: &inner.cfg.device_name,
            stamp: &stamp,
        })
    };
    let total = actions.iter().filter(|a| !matches!(a, Action::UpdateBase { .. })).count();
    let mut remaining = total;
    if total > 0 {
        inner.emit(EngineEvent::Status(SyncStatus::Syncing { pending: remaining }));
    }
    for action in actions {
        match action {
            Action::UpdateBase { path, hash } => {
                inner.index.lock().unwrap().set_base(&peer.device_id, &path, &hash)?;
                continue;
            }
            Action::Download { local_path, remote_path, hash, expected_local } => {
                download_one(inner, peer, out, pending, &local_path, &remote_path, hash, expected_local, given_up).await?;
            }
            Action::Conflict { local_path, conflict_path, remote_path, hash } => {
                let root = &inner.cfg.sync_dir;
                tokio::fs::rename(to_fs_path(root, &local_path), to_fs_path(root, &conflict_path))
                    .await
                    .with_context(|| format!("không đổi tên được {local_path}"))?;
                // quét lại: local_path thành "đã xoá", file conflict là file mới → báo cho máy kia
                inner.rescan().await?;
                inner.emit(EngineEvent::ConflictCreated { path: conflict_path });
                download_one(inner, peer, out, pending, &local_path, &remote_path, hash, None, given_up).await?;
            }
        }
        remaining -= 1;
        inner.emit(EngineEvent::Status(SyncStatus::Syncing { pending: remaining }));
    }
    if total > 0 {
        inner.emit_status();
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_one(inner: &Arc<Inner>, peer: &Peer, out: &mpsc::UnboundedSender<Out>, pending: &Pending,
                      local_path: &str, remote_path: &str, hash: Hash, expected_local: Option<Hash>,
                      given_up: &mut HashSet<(String, Hash)>) -> Result<()> {
    let key = (remote_path.to_string(), hash);
    if given_up.contains(&key) {
        return Ok(());
    }
    for attempt in 1..=3 {
        let (tx, rx) = oneshot::channel();
        *pending.lock().unwrap() =
            Some(PendingDownload { remote_path: remote_path.to_string(), local_path: local_path.to_string(), tx });
        out.send(Out::Msg(Message::GetFile { path: remote_path.to_string(), hash }))
            .map_err(|_| anyhow!("mất kết nối"))?;
        let incoming = match rx.await.context("mất kết nối khi đang tải file")? {
            DownloadResult::Unavailable => return Ok(()), // máy kia sẽ gửi IndexUpdate mới
            DownloadResult::Received(i) => i,
        };
        match incoming.finish(expected_local).await {
            Ok(entry) => {
                {
                    let mut idx = inner.index.lock().unwrap();
                    idx.record_file(&entry)?;
                    idx.set_base(&peer.device_id, &entry.path, &hash)?;
                }
                inner.broadcast_local_changes(vec![entry.clone()]);
                inner.emit(EngineEvent::FileSynced { path: entry.path });
                return Ok(());
            }
            Err(FinishError::LocalChanged) => return Ok(()), // vòng sau lập kế hoạch lại
            Err(FinishError::HashMismatch) if attempt < 3 => {
                tracing::warn!("hash không khớp khi tải {local_path}, thử lại lần {}", attempt + 1);
            }
            Err(FinishError::HashMismatch) => {
                given_up.insert(key);
                inner.emit(EngineEvent::Error(format!("Bỏ qua {local_path}: dữ liệu không khớp sau 3 lần thử")));
                return Ok(());
            }
            Err(e) => {
                inner.emit(EngineEvent::Error(format!("Không tải được {local_path}: {e}")));
                return Ok(());
            }
        }
    }
    Ok(())
}
```

- [x] **Step 6: Chạy test, xác nhận PASS**

Run: `cargo test -p lansync-core --test sync`
Expected: `1 passed`. Nếu fail, bật log để xem lý do: `RUST_LOG=lansync_core=debug cargo test -p lansync-core --test sync -- --nocapture` (cần thêm `tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).try_init().ok();` ở đầu test).

- [x] **Step 7: Chạy toàn bộ test + clippy**

Run: `cargo test -p lansync-core && cargo clippy -p lansync-core --all-targets -- -D warnings`
Expected: tất cả PASS, không có warning.

- [x] **Step 8: Commit**

```bash
git add crates/lansync-core
git commit -m "feat(core): engine với ghép cặp, phiên đồng bộ và test tích hợp"
```

---

### Task 10: Test tích hợp cho các quy tắc quan trọng

Các test này kiểm tra tiêu chí thành công 3, 4, 5 của spec cùng quy tắc không đồng bộ việc xoá. Engine đã được thiết kế cho các trường hợp này, nên test có thể PASS ngay. Nếu test nào FAIL thì đó là bug của Task 9, phải sửa trong `engine.rs`/`planner.rs`, **không sửa test**.

**Files:**
- Modify: `crates/lansync-core/tests/sync.rs` (thêm test vào cuối file)

- [x] **Step 1: Thêm test**

```rust
fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with(".lansync-tmp-")).collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_is_not_propagated_and_not_redownloaded() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Engine::start(cfg(tmp.path(), "mac")).await.unwrap();
    let b = Engine::start(cfg(tmp.path(), "win")).await.unwrap();
    std::fs::write(a.sync_dir().join("x.txt"), b"x").unwrap();
    pair(&a, &b).await;
    let bx = b.sync_dir().join("x.txt");
    assert!(wait_until(15, || bx.exists()).await);

    std::fs::remove_file(&bx).unwrap();
    tokio::time::sleep(Duration::from_secs(6)).await; // > 2 lần rescan_interval
    assert!(a.sync_dir().join("x.txt").exists(), "xoá ở Win không được làm mất file ở Mac");
    assert!(!bx.exists(), "Win không được tự tải lại file đã xoá");

    // nhưng nếu Mac sửa file đó thì Win nhận lại bản mới
    std::fs::write(a.sync_dir().join("x.txt"), b"ban sua moi").unwrap();
    assert!(wait_until(15, || read(&bx).as_deref() == Some(b"ban sua moi")).await);
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_edits_offline_keep_both_versions_on_both_machines() {
    let tmp = tempfile::tempdir().unwrap();
    let (ca, cb) = (cfg(tmp.path(), "mac"), cfg(tmp.path(), "win"));
    let a = Engine::start(ca.clone()).await.unwrap();
    let b = Engine::start(cb.clone()).await.unwrap();
    std::fs::write(a.sync_dir().join("doc.txt"), b"goc").unwrap();
    pair(&a, &b).await;
    assert!(wait_until(15, || read(&b.sync_dir().join("doc.txt")).as_deref() == Some(b"goc")).await);
    tokio::time::sleep(Duration::from_secs(1)).await; // để base hash được cập nhật ở cả hai bên
    let (id_a, id_b) = (a.device_id().to_string(), b.device_id().to_string());
    a.shutdown().await;
    b.shutdown().await;

    // sửa khác nhau khi đang mất kết nối
    std::fs::write(ca.sync_dir.join("doc.txt"), b"ban cua mac").unwrap();
    std::fs::write(cb.sync_dir.join("doc.txt"), b"ban cua windows dai hon").unwrap();

    let a = Engine::start(ca.clone()).await.unwrap();
    let b = Engine::start(cb.clone()).await.unwrap();
    b.connect_peer(addr(&a)); // cổng mới sau khi khởi động lại

    let (winner_content, loser_content, loser_name): (&[u8], &[u8], &str) =
        if id_a < id_b { (b"ban cua mac", b"ban cua windows dai hon", "win") }
        else { (b"ban cua windows dai hon", b"ban cua mac", "mac") };

    let ok = wait_until(20, || {
        let (fa, fb) = (files_in(&ca.sync_dir), files_in(&cb.sync_dir));
        fa.len() == 2 && fa == fb
            && read(&ca.sync_dir.join("doc.txt")).as_deref() == Some(winner_content)
            && read(&cb.sync_dir.join("doc.txt")).as_deref() == Some(winner_content)
    }).await;
    let fa = files_in(&ca.sync_dir);
    assert!(ok, "Mac: {fa:?} — Win: {:?}", files_in(&cb.sync_dir));
    let conflict = fa.iter().find(|n| n.starts_with(&format!("doc (conflict-{loser_name} "))).expect("thiếu file conflict");
    assert_eq!(read(&ca.sync_dir.join(conflict)).as_deref(), Some(loser_content));
    assert_eq!(read(&cb.sync_dir.join(conflict)).as_deref(), Some(loser_content));
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unpaired_device_cannot_exchange_files() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Engine::start(cfg(tmp.path(), "mac")).await.unwrap();
    let c = Engine::start(cfg(tmp.path(), "la")).await.unwrap();
    std::fs::write(c.sync_dir().join("spam.txt"), b"x").unwrap();
    std::fs::write(a.sync_dir().join("bi-mat.txt"), b"y").unwrap();
    c.connect_peer(addr(&a));
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!a.sync_dir().join("spam.txt").exists());
    assert!(!c.sync_dir().join("bi-mat.txt").exists());
    assert!(a.online_peers().is_empty());
    a.shutdown().await;
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnects_after_restart_and_syncs_missing_files() {
    let tmp = tempfile::tempdir().unwrap();
    let (ca, cb) = (cfg(tmp.path(), "mac"), cfg(tmp.path(), "win"));
    let a = Engine::start(ca.clone()).await.unwrap();
    let b = Engine::start(cb.clone()).await.unwrap();
    pair(&a, &b).await;
    b.shutdown().await;

    std::fs::write(ca.sync_dir.join("trong-luc-tat.txt"), b"z").unwrap();
    let b = Engine::start(cb.clone()).await.unwrap();
    a.connect_peer(addr(&b));
    assert!(wait_until(15, || read(&cb.sync_dir.join("trong-luc-tat.txt")).as_deref() == Some(b"z")).await);
    a.shutdown().await;
    b.shutdown().await;
}

// File lớn: chạy tay với `cargo test -p lansync-core --release --test sync -- --ignored`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn large_file_200mb() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let a = Engine::start(cfg(tmp.path(), "mac")).await.unwrap();
    let b = Engine::start(cfg(tmp.path(), "win")).await.unwrap();
    let src = a.sync_dir().join("lon.bin");
    {
        let mut f = std::fs::File::create(&src).unwrap();
        let block: Vec<u8> = (0..1_048_576u32).map(|i| (i % 253) as u8).collect();
        for i in 0..200u32 {
            f.write_all(&i.to_le_bytes()).unwrap();
            f.write_all(&block).unwrap();
        }
    }
    let want = lansync_core::fsutil::hash_file(&src).unwrap();
    pair(&a, &b).await;
    let dst = b.sync_dir().join("lon.bin");
    assert!(wait_until(120, || lansync_core::fsutil::hash_file(&dst).ok() == Some(want)).await);
    a.shutdown().await;
    b.shutdown().await;
}
```

- [x] **Step 2: Chạy test**

Run: `cargo test -p lansync-core --test sync`
Expected: `5 passed; 1 ignored`.
Run: `cargo test -p lansync-core --release --test sync -- --ignored`
Expected: `1 passed`.

Nếu FAIL: dùng skill superpowers:systematic-debugging, sửa trong `src/`, không nới lỏng test.

- [x] **Step 3: Commit**

```bash
git add crates/lansync-core/tests/sync.rs
git commit -m "test(core): xoá không lan sang máy kia, xung đột, máy lạ, kết nối lại, file lớn"
```

---

### Task 11: Công cụ dev + README + test tay Mac ⇄ Windows

App thật chạy ở khay hệ thống sẽ làm ở Plan 2. Example `dev` dưới đây chỉ là công cụ để **thử lõi trên 2 máy thật** trước khi làm giao diện.

**Files:**
- Create: `crates/lansync-core/examples/dev.rs`, `README.md`

- [x] **Step 1: Viết example**

`crates/lansync-core/examples/dev.rs`:
```rust
//! Công cụ dev để thử lõi LanSync trên 2 máy thật (chưa phải app chính thức).
//! Chạy: cargo run -p lansync-core --example dev --release
use lansync_core::{Config, Engine};
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,lansync_core=debug".into()),
        )
        .init();
    let h = Engine::start(Config::default_for_os()?).await?;
    println!("LanSync dev — máy: {} — id: {} — cổng: {}", h.device_name(), h.device_id(), h.local_port());
    println!("Thư mục đồng bộ: {}", h.sync_dir().display());
    println!("Lệnh: found | peers | pair <ip:port> | yes | no | connect <ip:port> | unpair <id> | quit");

    let mut events = h.subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            println!("[sự kiện] {ev:?}");
        }
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["found"] => {
                for d in h.discovered_peers() {
                    println!("  {} ({}) {:?}", d.name, d.device_id, d.addrs);
                }
            }
            ["peers"] => {
                let online = h.online_peers();
                for p in h.paired_peers() {
                    let mark = if online.contains(&p.device_id) { "●" } else { "○" };
                    println!("  {mark} {} ({}) {:?}", p.name, p.device_id, p.last_addr);
                }
            }
            ["pair", a] | ["connect", a] => match a.parse::<SocketAddr>() {
                Ok(addr) if parts[0] == "pair" => {
                    let h2 = h.clone();
                    tokio::spawn(async move {
                        if let Err(e) = h2.pair(addr).await {
                            println!("ghép cặp lỗi: {e:#}");
                        }
                    });
                }
                Ok(addr) => h.connect_peer(addr),
                Err(e) => println!("địa chỉ không hợp lệ: {e}"),
            },
            ["yes"] => h.confirm_pairing(true),
            ["no"] => h.confirm_pairing(false),
            ["unpair", id] => {
                if let Err(e) = h.unpair(id) {
                    println!("lỗi: {e:#}");
                }
            }
            ["quit"] => break,
            [] => {}
            _ => println!("lệnh không hợp lệ"),
        }
    }
    h.shutdown().await;
    Ok(())
}
```

- [x] **Step 2: Build thử**

Run: `cargo build -p lansync-core --example dev --release`
Expected: build thành công, không có warning.

- [x] **Step 3: Viết `README.md`**

```markdown
# LanSync

Đồng bộ thư mục `LanSync` giữa Mac và Windows qua mạng LAN. Không cần internet, không qua cloud.

## Trạng thái
- [x] Plan 1: lõi `lansync-core` + công cụ dev
- [x] Plan 2: app chạy ở khay hệ thống (Tauri)

## Thử nhanh bằng công cụ dev
1. Cài Rust 1.98+ trên cả hai máy (https://rustup.rs).
2. Trên mỗi máy: `cargo run -p lansync-core --example dev --release`
3. Gõ `found` để thấy máy kia (cần chung mạng Wi-Fi/LAN).
4. Trên một máy: `pair <ip-máy-kia>:47800`. Hai máy hiện cùng mã 6 số → gõ `yes` trên **cả hai** máy.
5. Bỏ file vào `~/LanSync` (Mac) hoặc `%USERPROFILE%\LanSync` (Windows).

## Quy tắc
- File mới và file sửa được đồng bộ 2 chiều.
- **Xoá file không được đồng bộ.** Xoá ở máy nào thì chỉ mất ở máy đó.
- Hai máy cùng sửa một file → giữ cả hai bản, một bản được đổi tên thành `tên (conflict-<máy> ...)`.
- Không đồng bộ: `.DS_Store`, `Thumbs.db`, `desktop.ini`, `~$*`, `*.tmp`, `._*`, symlink, tên không hợp lệ trên Windows.

## Sự cố thường gặp
- **Không thấy máy kia:** Wi-Fi công cộng/khách thường bật "client isolation". Dùng mạng nhà hoặc nhập IP thủ công.
- **Windows Firewall hỏi quyền:** chọn **Private networks** → Allow.
- **macOS hỏi "Local Network":** chọn Allow (System Settings → Privacy & Security → Local Network).

## Phát triển
- Test: `cargo test -p lansync-core`
- Test file lớn: `cargo test -p lansync-core --release --test sync -- --ignored`
- Spec: `docs/superpowers/specs/2026-10-13-lansync-design.md`
```

- [x] **Step 4: Test tay Mac ⇄ Windows** (ghi kết quả từng mục vào commit message hoặc báo lại cho user)

- [x] Mac và Windows cùng chạy `cargo run -p lansync-core --example dev --release`. Windows Firewall: Allow (Private). macOS Local Network: Allow.
- [x] `found` trên mỗi máy thấy máy còn lại.
- [x] Mac gõ `pair <ip-windows>:47800` → hai máy hiện **cùng** mã 6 số → `yes` trên cả hai → `PairingResult { ok: true }` trên cả hai.
- [x] Copy file 1 GB vào `~/LanSync` trên Mac. Kiểm tra hash: Mac `shasum -a 256 ~/LanSync/file` và Windows `certutil -hashfile %USERPROFILE%\LanSync\file SHA256` cho cùng kết quả.
- [x] Sửa một file `.txt` trên Windows rồi lưu → Mac có bản mới trong ≤ 5 giây.
- [x] File tên tiếng Việt `Tiếng Việt có dấu.txt` từ Mac → Windows hiện đúng tên.
- [x] Xoá một file trên Mac → Windows vẫn còn file đó; chờ 1 phút, Mac không tự tải lại file.
- [x] Ngắt Wi-Fi trên Windows, sửa cùng một file khác nhau trên cả hai máy, bật Wi-Fi lại → cả hai máy có `file.txt` và `file (conflict-...).txt`.
- [x] `quit` trên Windows, thêm file trên Mac, chạy lại trên Windows → tự kết nối lại và nhận file, không cần gõ lệnh.
- [x] Trên Mac tạo file `a:b.txt` → có sự kiện `Skipped`, Windows không nhận file này.
- [x] Máy thứ ba (hoặc xoá `peers.json` trên một máy) không đồng bộ được cho tới khi ghép cặp lại.

- [x] **Step 5: Commit**

```bash
git add README.md crates/lansync-core/examples/dev.rs
git commit -m "feat(core): công cụ dev để test tay + README"
```

---

## Sau Plan 1

Khi toàn bộ checklist test tay ở Task 11 đạt, viết **Plan 2: `lansync-tray`** (Tauri v2: menu khay, cửa sổ ghép cặp, autostart, log xoay vòng, `Info.plist` cho Local Network, luật firewall trong `.msi`, đóng gói `.dmg`/`.msi`), dựa trên mục 5.11–5.12 của spec.
