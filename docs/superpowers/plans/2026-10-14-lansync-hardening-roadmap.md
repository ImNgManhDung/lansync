# LanSync — Roadmap Hardening & Tính năng mở rộng

> Nguồn: review thiết kế v1. Module hiện có: `crates/lansync-core/src/{index,planner,watcher,transfer,engine,paths,fsutil,pairing,identity,discovery,transport,protocol,config}.rs`, `apps/lansync-tray/src/{tray,commands,state}.rs`.
> Quy ước: `[ ]` chưa làm · `[x]` xong. Mỗi task = test trước (TDD) → code → `cargo test` xanh.
> Ưu tiên: **P0** chặn phát hành · **P1** nên có v1.0 · **P2** v1.x · **P3** sau.

---

## Thứ tự thực hiện đề xuất

| Giai đoạn | Nội dung | Mục tiêu |
|---|---|---|
| **A (P0)** | 1.1, 1.3, 1.5, Nhóm 4 (path traversal, brute-force, revoke), ổ rút ra (5.5) | Không mất dữ liệu, không bị tấn công |
| **B (P1)** | 1.2, 1.4, 2.1, 2.3, 2.5, 3.1, 3.2, 3.4, 3.6, 3.7, 5.x onboarding, 6.x | Dùng thực tế ổn định |
| **C (P2)** | 2.4, 2.6, 3.3, 3.5, Keychain/DPAPI, Trusted networks, updater | Hoàn thiện |
| **D (P3)** | 2.2 Delta/FastCDC, Linux, Mobile, WoL, mã hoá at-rest | Mở rộng |

---

## 🔴 Nhóm 1 — Lỗ hổng logic (P0)

### 1.1 Tombstone chống file "sống lại"  `index.rs`, `planner.rs`, `watcher.rs`
- [x] Migration SQLite: bảng `tombstones(path, hash, deleted_at)`
- [x] Index/Watcher: khi phát hiện xoá → ghi tombstone kèm last known hash
- [x] Planner: bỏ qua pull nếu peer có `path` + `hash` khớp tombstone
- [x] Peer có hash mới → coi là phiên bản mới, pull bình thường
- [x] Hành động thủ công "Restore from peer" (`restore_tombstone` API)
- [x] Test: xoá A → sync lại → A không hồi sinh; sửa ở B → A nhận bản mới; restore thủ công OK

### 1.2 Delete mode theo peer  `config.rs`, `planner.rs`, `engine.rs`
- [ ] Config: `delete_mode = never | ask | mirror_trash` mỗi peer (mặc định `never`)
- [ ] Propagate tombstone sang peer qua protocol (message `DeleteNotice{paths, hashes}`)
- [ ] `ask`: tạo pending-deletion queue + thông báo "Mac đã xoá 12 file, xoá ở đây?"
- [ ] `mirror_trash`: chuyển vào Trash hệ điều hành (crate `trash`), không xoá vĩnh viễn
- [ ] UI tray: hộp thoại xác nhận / chọn mode trong cài đặt peer
- [ ] Test: 3 mode × (xoá file, xoá thư mục), xác nhận không xoá vĩnh viễn

### 1.3 Version vector thay mtime  `index.rs`, `planner.rs`, `protocol.rs`
- [x] Cột `vv` (map `device_id → counter`) trên mỗi file entry
- [x] Tăng counter của máy mình khi watcher thấy thay đổi cục bộ
- [x] Trao đổi `vv` trong index exchange
- [x] Planner: so sánh vv → `newer | older | equal | concurrent(conflict)`
- [x] mtime chỉ dùng hiển thị; tên file conflict dùng device name + counter thay vì timestamp
- [x] Migration dữ liệu cũ (vv khởi tạo từ index hiện tại)
- [x] Test: randomized property test cho vv; kịch bản đồng hồ lệch ±1 giờ/±1 ngày

### 1.4 Phát hiện rename/move  `watcher.rs`, `index.rs`, `planner.rs`, `engine.rs`
- [x] Planner: phát hiện rename/move bằng đối sánh hash + kiểm tra file cũ bị xóa trên remote
- [x] Engine: thực thi `Action::Rename`, đổi tên file cục bộ trực tiếp (0 byte truyền tải qua mạng)
- [x] Tự động tạo thư mục cha khi di chuyển file sang thư mục mới
- [x] Cập nhật index và set base hash cho file đã đổi tên
- [x] Test: E2E đổi tên file trên Machine A → Machine B đổi tên tương ứng mà không truyền lại payload

### 1.5 Khác biệt filesystem Mac ↔ Windows  `paths.rs`, `fsutil.rs`
- [x] Chuẩn hoá NFC khi lưu/so sánh path (`unicode-normalization`)
- [x] Phát hiện ký tự cấm Windows `: * ? " < > |`, tên reserved (CON, NUL, COM1…), tên kết thúc `.`/space
- [x] Tự đổi tên an toàn cho Windows (`sanitize_for_windows`)
- [x] Phát hiện va chạm hoa/thường (`resolve_case_collision`) → tạo file conflict an toàn, không ghi đè
- [x] Unit test kiểm tra đầy đủ các kịch bản tên Windows và va chạm hoa/thường

---

## 🟠 Nhóm 2 — Độ tin cậy & hiệu năng

### 2.1 Resume truyền dở (P1)  `transfer.rs`, `protocol.rs`
- [ ] Chia chunk 4 MB, bitmap chunk nhận lưu cạnh `.lansync-tmp-*`
- [ ] Protocol: `RangeRequest{path, chunks[]}`; xin phần thiếu khi nối lại
- [ ] Xác minh hash toàn file trước rename atomic
- [ ] Dọn temp mồ côi sau 24h (job nền)
- [ ] Test: cắt kết nối ở 95% → resume chỉ tải 5%; temp hỏng → tải lại

### 2.2 Delta sync / FastCDC (P3)
- [ ] Spike: benchmark FastCDC + BLAKE3 trên file VM 30GB
- [ ] Block index per-file; protocol `BlockList` + `BlockRequest`
- [ ] Chỉ kéo chunk khác; fallback full-copy cho file nhỏ
- [ ] Test: sửa 1 dòng/1 block → truyền < 1% dung lượng

### 2.3 File đang ghi/khoá (P1)  `watcher.rs`, `fsutil.rs`
- [x] Debounce ổn định: hash_file_stable xác minh mtime và file size ổn định trước và sau khi đọc
- [x] Thử mở exclusive/shared-read; bị khoá → open_with_retry backoff
- [x] Hiển thị thông tin file được ghi / tải trong Activity
- [ ] (P3) Windows VSS cho file bị khoá lâu
- [x] Test: file hash stable, retry open file

### 2.4 Truyền song song (P2)  `transfer.rs`, `transport.rs`
- [ ] Pool 4–8 stream đồng thời (cấu hình)
- [ ] Batch file nhỏ (< 256 KB) vào 1 request/archive stream
- [ ] Benchmark: 10.000 file × 4KB trước/sau; mục tiêu ≥ 5× nhanh hơn
- [ ] Giữ thứ tự ưu tiên (file nhỏ/đang mở trước)

### 2.5 Scrub & full rescan (P1)  `engine.rs`, `index.rs`
- [x] Full rescan khi khởi động nếu lần tắt trước không sạch (cờ `clean_shutdown`)
- [ ] Phát hiện overflow buffer FSEvents/ReadDirectoryChangesW → trigger rescan thư mục
- [ ] Scrub hash theo lịch (tuần/tháng, cấu hình), throttle I/O
- [ ] Báo cáo mismatch hash không kèm thay đổi mtime (bit rot) → cảnh báo, không tự ghi đè
- [x] Test: mô phỏng clean shutdown flag

### 2.6 Giới hạn băng thông & Tạm dừng (P1)  `transfer.rs`, `engine.rs`, `commands.rs`, `app.js`
- [x] Giới hạn tốc độ tải / upload (chunk pacing chính xác ±1% theo KB/s)
- [x] Tạm dừng / Tiếp tục đồng bộ (Pause / Resume sync)
- [x] UI điều khiển Tạm dừng & chọn Giới hạn băng thông trong Cài đặt
- [x] Test: kiểm tra pause state và bandwidth limit setting trong engine

---

## 🟡 Nhóm 3 — Tính năng người dùng

### 3.1 File versioning (P1)  `engine.rs`, `versioning.rs`, `commands.rs`, `app.js`
- [x] Khi ghi đè bằng bản kéo về → chuyển bản cũ vào `.lansync-versions/<path>/<ts>_<hash>`
- [x] Retention: giữ 5 bản gần nhất cho mỗi file, tự động prune
- [x] Module `versioning.rs` + tích hợp vào `transfer.rs::Incoming::finish`
- [x] UI "Phiên bản": duyệt danh sách file và khôi phục (Restore previous version) chỉ bằng 1 cú nhấp
- [x] Test: ghi đè → bản cũ còn; vượt quota → xoá bản cũ nhất; khôi phục thành công qua timestamp

### 3.2 Selective sync & ignore (P1)  `config.rs`, `watcher.rs`, `planner.rs`
- [x] Parser `.lansyncignore` trong root folder
- [x] Mặc định ignore: `node_modules`, `.git`, `*.tmp`, `*.part`, `*.crdownload`, `.lansync-*`
- [ ] Mode theo thư mục: `two-way | receive-only | send-only`
- [ ] UI chọn thư mục con
- [x] Test: ignore áp dụng cả watcher + planner + index exchange

### 3.3 Nhiều thư mục & nhiều peer (P2)  `config.rs`, `engine.rs`, `index.rs`
- [ ] Model `SyncPair{id, folder, peers[]}`; migration từ config 1-1
- [ ] Mỗi pair có engine/index riêng
- [ ] Pull đa nguồn: chọn peer online, chia chunk giữa nhiều peer (swarm nội bộ)
- [ ] Gossip index giữa N peer, tránh vòng lặp (dùng vv)
- [ ] Test: 3 máy A/B/C, A sửa → B,C đồng bộ; C offline rồi online

### 3.4 Activity Log & Conflicts (P1)  `index.rs`, `commands.rs`, `tray.rs`
- [x] Event stream & Activity log hiển thị trên tab Lịch sử
- [x] Màn Conflicts: Phát hiện xung đột, hiển thị danh sách file xung đột
- [x] Các nút giải quyết xung đột: Giữ bản máy này (Keep mine) / Giữ bản máy kia (Keep theirs) / Giữ cả hai (Keep both)
- [x] Command Tauri `list_conflicts` và `resolve_conflict`

### 3.5 Quick Send / Drop (P2)  `pairing.rs`, `transfer.rs`, `tray.rs`
- [ ] Protocol `OneShotOffer{name,size}` / accept-reject, dùng chung mTLS
- [ ] Kéo thả vào icon tray → chọn thiết bị → gửi
- [ ] Peer chưa ghép: pairing tạm thời (SAS), không lưu vĩnh viễn
- [ ] Thư mục nhận riêng (`Downloads/LanSync`)

### 3.6 Autostart & chạy nền (P1)  `apps/lansync-tray`
- [ ] Login Item (macOS) / Task Scheduler hoặc Registry Run (Windows), toggle trong settings
- [ ] Watchdog tự restart khi crash
- [ ] Phát hiện wake from sleep / đổi mạng → re-discover + reconnect
- [ ] Test thủ công trên cả 2 OS (checklist ghi vào QA)

### 3.7 Icon trạng thái & thông báo (P1)  `tray.rs`
- [ ] 4 icon: xanh (synced) / xoay (đang chạy) / vàng (conflict) / đỏ (lỗi)
- [ ] Notification khi nhận xong file lớn (>100MB) & khi có conflict
- [ ] Cho phép tắt/bật từng loại thông báo

---

## 🔵 Nhóm 4 — Bảo mật

### P0
- [x] **Path traversal**: validate mọi path nhận từ peer — cấm `..`, absolute, drive letter, symlink thoát root; canonicalize + kiểm tra prefix (`paths.rs`, `transfer.rs`)
- [ ] Fuzz test `paths::sanitize` (cargo-fuzz) + test `../../Windows/...`
- [x] **Brute-force pairing**: mã sống 60–120s, tối đa 3–5 lần thử, khoá phiên + backoff (`pairing.rs`)
- [ ] Cân nhắc SPAKE2 (crate `spake2`) hoặc SAS hiển thị 2 phía thay mã 6 số thuần
- [x] **Revoke/Forget device**: xoá khỏi `peers.json`, ngắt kết nối đang mở (`engine.unpair_peer`)

### P1–P2
- [ ] Rotate chứng chỉ/khoá định kỳ + khi revoke
- [ ] Quota & anomaly: giới hạn size/số file mỗi lần; cảnh báo + tạm dừng khi nhận bất thường (vd >1000 file lạ/5 phút)
- [ ] Khoá riêng Ed25519 → Keychain (macOS, `security-framework`) / DPAPI-Credential Manager (Windows) (`identity.rs`); migrate từ file
- [ ] Trusted networks: chỉ bật mDNS/accept trên SSID/subnet đã đánh dấu
- [ ] (P3) Mã hoá at-rest cho `.lansync-versions/` và SQLite index (SQLCipher hoặc age)

---

## 🟣 Nhóm 5 — Kết nối & nền tảng

- [x] **(P0) Ổ rút ra / thư mục gốc biến mất**: kiểm tra root tồn tại + marker `.lansync-root` trước mỗi scan; mất root → tạm dừng, KHÔNG tạo tombstone; test mô phỏng unmount
- [ ] (P1) Onboarding: màn hướng dẫn quyền Local Network (macOS 15+) & Windows Firewall; nút "Không thấy máy?" kiểm tra chẩn đoán
- [ ] (P1) Chọn interface ưu tiên, bỏ adapter ảo (VMware, WSL, Docker, Tailscale) trong `discovery.rs`
- [ ] (P1) Fallback khi mDNS bị chặn: nhập IP, mã thiết bị (device ID), QR code, quét subnet; hỗ trợ IPv6 link-local
- [ ] (P2) Hàng đợi khi peer offline: tự đồng bộ khi online (kèm retry); (P3) Wake-on-LAN
- [ ] (P3) Linux build (Tauri) + CI
- [ ] (P3) Mobile Android/iOS (ưu tiên upload ảnh/video)
- [ ] (P2) Ổ mạng (SMB/NFS): phát hiện watcher không tin cậy → chuyển sang polling

---

## 🟤 Nhóm 6 — Vận hành & chất lượng

- [ ] (P1) Diagnostics: "Export logs" (zip, che dữ liệu nhạy cảm), debug mode, màn "Vì sao file này chưa đồng bộ?"
- [ ] (P1) Cảnh báo dung lượng ổ trước khi pull (free space ≥ size + 10% dự phòng)
- [ ] (P1) i18n Việt/Anh ngay từ đầu (fluent hoặc rust-i18n), toàn bộ chuỗi UI
- [ ] (P2) Auto-update: Tauri updater + chữ ký; macOS notarization, Windows code signing; pipeline CI release
- [ ] (P1) Telemetry mặc định TẮT; chỉ crash report cục bộ
- [ ] (P1) Bộ test tích hợp: mạng đứt, rename đồng thời, conflict 3 chiều, đồng hồ lệch, file Unicode
- [ ] (P1) Property-based test cho planner (proptest)
- [ ] (P2) Fuzz giao thức (`protocol.rs` decode)
- [ ] (P2) CI matrix macOS + Windows (+ Linux)

---

## Definition of Done cho release v1.0

- [ ] Toàn bộ P0 xong và có test
- [ ] Kịch bản "xoá file", "rút ổ USB", "đồng hồ lệch", "tên tiếng Việt Mac↔Win" chạy E2E không mất/hồi sinh dữ liệu
- [ ] Không còn đường dẫn nhận từ peer nào bỏ qua `sanitize`
- [ ] `cargo test --workspace` + `clippy -D warnings` xanh trên macOS & Windows
- [ ] Test thủ công 2 máy thật (Mac ↔ Win) theo checklist QA
