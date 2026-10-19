# LanSync Native GUI (`egui`) với Clean Architecture — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thay thế hoàn toàn Tauri v2 + HTML/CSS/JS trong `apps/lansync-tray` bằng ứng dụng Rust Native GUI (`eframe`/`egui` + `tray-icon`) được thiết kế chuẩn **Clean Architecture** (`domain/`, `application/`, `infrastructure/`, `presentation/`), giữ nguyên 100% `crates/lansync-core`.

**Architecture:** `apps/lansync-tray` được phân tách thành 4 tầng độc lập theo Dependency Inversion Principle:
1. `domain`: Chứa Entities thuần (`PeerEntity`, `ConflictEntity`, `ActivityEntity`, ...) và các Ports (`SyncBackendPort`, `SystemServicePort`).
2. `application`: Chứa `AppStateSnapshot`, `AppCommand`, và `AppUseCaseInteractor` điều phối luồng dữ liệu bất đồng bộ.
3. `infrastructure`: Chứa `LanSyncCoreAdapter` (bọc `lansync-core`), `OsSystemAdapter` (`rfd`, `open`, `arboard`), và `SystemTrayAdapter` (`tray-icon`, `muda`).
4. `presentation`: Giao diện Desktop Dashboard (`eframe`/`egui`) với Sidebar 5 tab và bộ nạp font tiếng Việt tự động trên macOS & Windows.

**Tech Stack:** Rust 2024, `eframe = "0.31"`, `egui = "0.31"`, `tray-icon = "0.19"`, `muda = "0.15"`, `image = "0.25"`, `arboard = "3"`, `async-trait = "0.1"`, `rfd = "0.15"`, `open = "5"`, `tokio = "1"`.

**Spec:** `docs/superpowers/specs/2026-10-17-lansync-egui-design.md`

## Global Constraints

- Giữ nguyên 100% thư viện `crates/lansync-core` (không sửa đổi bất kỳ file nào trong `crates/lansync-core`).
- Tuân thủ nghiêm ngặt quy tắc phụ thuộc của Clean Architecture:
  - `domain/` không import `lansync_core`, `egui`, hay `tray_icon`.
  - `application/` chỉ import `domain/`.
  - `presentation/` chỉ import `application/` và `domain/`.
  - Chỉ `infrastructure/` và `main.rs` (Composition Root) được phép kết nối với `lansync_core`, `rfd`, `open`, `tray_icon`.
- Xóa toàn bộ cấu hình và mã nguồn Tauri/WebView cũ (`build.rs`, `tauri.conf.json`, `gen/`, `ui/`, `src/commands.rs`, `src/state.rs`, `src/tray.rs`).
- Toàn bộ workspace phải pass `cargo check --workspace` và `cargo test --workspace`.

---

### Task 1: Dọn bỏ Tauri & Xây dựng Tầng `domain/` (Entities, Value Objects & Ports)

**Files:**
- Delete: `apps/lansync-tray/build.rs`
- Delete: `apps/lansync-tray/tauri.conf.json`
- Delete: `apps/lansync-tray/gen/`
- Delete: `apps/lansync-tray/ui/`
- Delete: `apps/lansync-tray/src/commands.rs`
- Delete: `apps/lansync-tray/src/state.rs`
- Delete: `apps/lansync-tray/src/tray.rs`
- Modify: `apps/lansync-tray/Cargo.toml`
- Create: `apps/lansync-tray/src/domain/mod.rs`
- Create: `apps/lansync-tray/src/domain/models.rs`
- Create: `apps/lansync-tray/src/domain/ports.rs`

**Interfaces:**
- Produces:
  - `domain::models::{PeerEntity, ConflictEntity, ActivityEntity, PendingPairingEntity, PendingDeletionEntity, VersionSnapshotEntity, DeletePolicy, ConflictResolution, derive_original_from_conflict, format_bytes}`
  - `domain::ports::{SyncBackendPort, SystemServicePort, BackendEvent}`

- [x] **Step 1: Xóa các file Tauri và giao diện HTML/CSS/JS cũ**

Run:
```bash
rm -rf apps/lansync-tray/build.rs apps/lansync-tray/tauri.conf.json apps/lansync-tray/gen apps/lansync-tray/ui apps/lansync-tray/src/commands.rs apps/lansync-tray/src/state.rs apps/lansync-tray/src/tray.rs
```

- [x] **Step 2: Cập nhật `apps/lansync-tray/Cargo.toml`**

```toml
[package]
name = "lansync-tray"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
lansync-core = { path = "../../crates/lansync-core" }
eframe = { version = "0.31", default-features = true }
egui = "0.31"
tray-icon = "0.19"
muda = "0.15"
image = { version = "0.25", default-features = false, features = ["png"] }
arboard = "3"
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
open = "5"
directories = "6"
chrono = "0.4"
rand = "0.8"
rfd = "0.15"
walkdir = "2"
```

- [x] **Step 3: Tạo `apps/lansync-tray/src/domain/models.rs` và `apps/lansync-tray/src/domain/ports.rs` kèm Unit Tests cho logic Domain**

- [x] **Step 4: Chạy `cargo test -p lansync-tray` kiểm tra tầng `domain` biên dịch và pass unit test**

---

### Task 2: Xây dựng Tầng `application/` (State Store, Commands & UseCase Interactor)

**Files:**
- Create: `apps/lansync-tray/src/application/mod.rs`
- Create: `apps/lansync-tray/src/application/commands.rs`
- Create: `apps/lansync-tray/src/application/state.rs`
- Create: `apps/lansync-tray/src/application/use_cases.rs`

**Interfaces:**
- Consumes: `crate::domain::models::*`, `crate::domain::ports::*`
- Produces:
  - `application::commands::AppCommand`
  - `application::state::{AppStateSnapshot, AppStore}`
  - `application::use_cases::AppUseCaseInteractor`

- [x] **Step 1: Tạo `apps/lansync-tray/src/application/commands.rs` định nghĩa enum `AppCommand` cho mọi thao tác người dùng**

- [x] **Step 2: Tạo `apps/lansync-tray/src/application/state.rs` định nghĩa `AppStateSnapshot` và `AppStore` (`Arc<RwLock<AppStateSnapshot>>`)**

- [x] **Step 3: Tạo `apps/lansync-tray/src/application/use_cases.rs` xử lý `AppCommand` và `BackendEvent` thông qua `SyncBackendPort` + `SystemServicePort`, kèm Unit Test sử dụng Mock Port**

- [x] **Step 4: Chạy `cargo test -p lansync-tray` xác nhận UseCase Interactor hoạt động đúng với Mock Port mà không cần khởi tạo engine thật**

---

### Task 3: Xây dựng Tầng `infrastructure/` (Core Adapter, OS Adapter & System Tray Adapter)

**Files:**
- Create: `apps/lansync-tray/src/infrastructure/mod.rs`
- Create: `apps/lansync-tray/src/infrastructure/core_adapter.rs`
- Create: `apps/lansync-tray/src/infrastructure/os_adapter.rs`
- Create: `apps/lansync-tray/src/infrastructure/tray_adapter.rs`

**Interfaces:**
- Consumes: `crate::domain::*`, `lansync_core::*`, `tray_icon`, `muda`, `rfd`, `open`, `arboard`
- Produces:
  - `infrastructure::core_adapter::LanSyncCoreAdapter` (implements `SyncBackendPort`)
  - `infrastructure::os_adapter::OsSystemAdapter` (implements `SystemServicePort`)
  - `infrastructure::tray_adapter::SystemTrayAdapter`

- [x] **Step 1: Tạo `apps/lansync-tray/src/infrastructure/core_adapter.rs` bọc `Arc<SyncEngine>` và chuyển đổi `SyncEvent` sang `BackendEvent` của Domain**

- [x] **Step 2: Tạo `apps/lansync-tray/src/infrastructure/os_adapter.rs` triển khai mở thư mục, chọn thư mục (`rfd`), sao chép clipboard (`arboard`), và lấy IP LAN**

- [x] **Step 3: Tạo `apps/lansync-tray/src/infrastructure/tray_adapter.rs` quản lý icon khay hệ thống và menu chuột phải ("Mở thư mục LanSync", "Hiện bảng điều khiển", "Thoát LanSync")**

---

### Task 4: Xây dựng Tầng `presentation/` (Giao diện Desktop Dashboard `egui` / `eframe`)

**Files:**
- Create: `apps/lansync-tray/src/presentation/mod.rs`
- Create: `apps/lansync-tray/src/presentation/theme.rs`
- Create: `apps/lansync-tray/src/presentation/tabs/mod.rs`
- Create: `apps/lansync-tray/src/presentation/tabs/sync_tab.rs`
- Create: `apps/lansync-tray/src/presentation/tabs/devices_tab.rs`
- Create: `apps/lansync-tray/src/presentation/tabs/versions_tab.rs`
- Create: `apps/lansync-tray/src/presentation/tabs/activity_tab.rs`
- Create: `apps/lansync-tray/src/presentation/tabs/settings_tab.rs`

**Interfaces:**
- Consumes: `crate::application::*`, `crate::domain::models::*`, `egui`, `eframe`
- Produces: `presentation::LanSyncGuiApp` (implements `eframe::App`)

- [x] **Step 1: Tạo `apps/lansync-tray/src/presentation/theme.rs` nạp phông chữ tiếng Việt tự động cho macOS/Windows và bảng màu Dark Slate**

- [x] **Step 2: Tạo 5 tab trong `apps/lansync-tray/src/presentation/tabs/` (`sync_tab.rs`, `devices_tab.rs`, `versions_tab.rs`, `activity_tab.rs`, `settings_tab.rs`)**

- [x] **Step 3: Tạo `apps/lansync-tray/src/presentation/mod.rs` ghép nối Sidebar, Global Pairing Alert Banner (hiển thị mã 6 số), nội dung từng tab, Footer, và xử lý ẩn xuống System Tray khi bấm nút `X`**

---

### Task 5: Kết nối Composition Root (`main.rs`), Kiểm thử & Cập nhật Tài liệu

**Files:**
- Modify: `apps/lansync-tray/src/main.rs`
- Modify: `README.md`

- [x] **Step 1: Cập nhật `apps/lansync-tray/src/main.rs` khởi tạo các tầng theo Clean Architecture (`LanSyncCoreAdapter`, `OsSystemAdapter`, `AppUseCaseInteractor`, `SystemTrayAdapter`, `LanSyncGuiApp`)**

- [x] **Step 2: Chạy `cargo check --workspace` và `cargo test --workspace` xác nhận toàn bộ workspace biên dịch sạch và pass 100% unit/integration tests**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 3: Cập nhật `README.md` mô tả cấu trúc Clean Architecture của `apps/lansync-tray` và hướng dẫn sử dụng**
