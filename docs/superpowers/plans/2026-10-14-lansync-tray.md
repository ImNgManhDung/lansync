# Kế hoạch Hiện thực: LanSync System Tray App (Phase 2)

Kế hoạch này triển khai ứng dụng khay hệ thống (System Tray / Menu Bar) bằng **Tauri v2** (`apps/lansync-tray`) kết nối trực tiếp với thư viện lõi `lansync-core` đã hoàn thành ở Phase 1.

---

## 🎯 Mục tiêu
- Ứng dụng chạy ẩn dưới thanh Menu Bar (macOS) hoặc Khay hệ thống Taskbar (Windows).
- Bấm vào icon khay sẽ mở một cửa sổ Popover nhỏ gọn (360x520px), tự động ẩn khi click ra ngoài.
- Hiển thị trực quan:
  - Trạng thái đồng bộ (sẵn sàng, đang tải file kèm tên file/tiến độ, lỗi).
  - Nút bấm nhanh "Mở thư mục" (mở Finder trên macOS hoặc File Explorer trên Windows).
  - Danh sách thiết bị LAN tự động tìm thấy qua mDNS + trạng thái kết nối.
  - Hộp thoại xác nhận ghép cặp với mã 6 số to rõ ràng.
  - Màn hình cài đặt nhanh: đổi tên thiết bị, đổi thư mục đồng bộ, đổi cổng.
- Giao diện thiết kế cao cấp: Dark Slate Glassmorphism, Vanilla CSS, phông chữ Inter hiện đại, siêu nhẹ (<10MB RAM).

---

## 📋 Danh sách Task & Checklists

### Task 1: Thiết lập Crate `apps/lansync-tray` & Cấu hình Tauri v2
- [x] Thêm `apps/lansync-tray` vào `[workspace.members]` trong root `Cargo.toml`.
- [x] Tạo thư mục `apps/lansync-tray` với `Cargo.toml` chứa các dependencies:
  - `lansync-core = { path = "../../crates/lansync-core" }`
  - `tauri = { version = "2", features = ["tray-icon"] }`
  - `tauri-plugin-opener = "2"`
  - `tauri-plugin-dialog = "2"`
  - `serde`, `serde_json`, `tokio`, `tracing`, `tracing-subscriber`.
- [x] Tạo `apps/lansync-tray/build.rs` gọi `tauri_build::build()`.
- [x] Tạo `apps/lansync-tray/tauri.conf.json`:
  - `frontendDist`: `../ui`
  - Window: title `LanSync`, width 360, height 520, `decorations: false`, `resizable: false`, `visible: false`, `skipTaskbar: true`.
  - Bundle identifier: `com.lansync.tray`.
- [x] Tạo icon khay cơ bản dạng PNG trong `apps/lansync-tray/icons/`.
- [x] Kiểm tra lệnh `cargo check -p lansync-tray` thành công.

### Task 2: Backend IPC Commands & Cầu nối Sự kiện (Bridge)
- [x] Tạo `apps/lansync-tray/src/state.rs`:
  - `AppState`: lưu `Arc<SyncEngine>`, đường dẫn config, danh sách sự kiện gần nhất (activity log).
- [x] Tạo `apps/lansync-tray/src/commands.rs`:
  - `get_status`: trả về `AppStatus` (thông tin máy cục bộ, thư mục sync, danh sách peer).
  - `open_sync_folder`: mở thư mục trong Finder / Explorer qua native opener.
  - `pair_device`: kích hoạt ghép cặp với địa chỉ IP:Port hoặc Peer ID.
  - `approve_pairing`: đồng ý ghép cặp với mã 6 số.
  - `reject_pairing`: từ chối ghép cặp.
  - `update_config`: cập nhật tên máy, cổng, thư mục sync.
  - `pick_sync_folder`: mở hộp thoại chọn thư mục native.
- [x] Cầu nối sự kiện trong `main.rs`:
  - Lắng nghe `engine.subscribe_events()` và phát (emit) qua `app_handle.emit("sync_event", payload)` cho giao diện.
- [x] Viết unit tests kiểm thử các command logic trong `commands.rs`.

### Task 3: Quản lý System Tray & Hành vi Popover Window
- [x] Tạo `apps/lansync-tray/src/tray.rs`:
  - Khởi tạo `TrayIconBuilder` với icon LanSync.
  - Menu chuột phải: "Mở thư mục LanSync", "Cài đặt", "Thoát".
  - Bắt sự kiện click chuột trái vào Tray Icon:
    - Tính toán vị trí click và hiển thị popover window ngay dưới/trên tray icon.
    - Nếu cửa sổ đang mở thì ẩn đi (toggle).
  - Bắt sự kiện Window `Focused(false)`: tự động ẩn popover khi người dùng bấm ra ngoài màn hình.

### Task 4: Giao diện Popover - Cấu trúc HTML & Hệ thống Design (Vanilla CSS)
- [x] Tạo `apps/lansync-tray/ui/index.html`:
  - Header: Logo LanSync + Tên máy + Huy hiệu trạng thái (Online/Syncing) + Nút mở thư mục + Nút Cài đặt.
  - Card tiến trình: Hiển thị file đang tải/đồng bộ hoặc "Đã đồng bộ toàn bộ".
  - Danh sách Thiết bị: Thiết bị đã kết nối, thiết bị tìm thấy trong mạng LAN.
  - Banner ghép cặp: Hiển thị mã 6 số to, nút "Xác nhận ghép cặp" / "Từ chối".
  - Màn hình Settings trượt (Slide-in): Thư mục đồng bộ, tên máy, cổng mạng.
- [x] Tạo `apps/lansync-tray/ui/style.css`:
  - Bảng màu Dark Slate / Glassmorphism hiện đại (background blur, border gradient tinh tế).
  - Typography: Google Font Inter, kích thước rõ ràng, không dùng emoji dư thừa.
  - Micro-animations: Hiệu ứng chuyển động mượt mà khi xuất hiện modal, hiệu ứng pulse khi đang đồng bộ.
  - Hoàn toàn thuần Vanilla CSS (không dùng Tailwind để đảm bảo kiểm soát tối đa và tốc độ tải tức thì).

### Task 5: Logic Giao diện Frontend & Tương tác Thời gian thực (Vanilla JS)
- [x] Tạo `apps/lansync-tray/ui/app.js`:
  - Khởi tạo kết nối với Tauri IPC (`@tauri-apps/api/core`).
  - Lắng nghe sự kiện `sync_event`:
    - Khi có thiết bị mới qua mDNS -> cập nhật danh sách thiết bị.
    - Khi có yêu cầu ghép cặp -> hiển thị modal mã 6 số kèm rung nhẹ thông báo.
    - Khi có file đang tải (`FileDownloading`) -> hiện progress bar và tên file.
    - Khi hoàn tất (`SyncCompleted`) -> đổi trạng thái sang "Đã đồng bộ".
  - Xử lý click:
    - Nút "Mở thư mục" -> gọi IPC `open_sync_folder`.
    - Nút "Ghép cặp" -> gọi IPC `approve_pairing` / `reject_pairing`.
    - Nút "Lưu cài đặt" -> gọi IPC `update_config`.
    - Nút "Chọn thư mục" -> gọi IPC `pick_sync_folder`.

### Task 6: Kiểm thử Tích hợp & Đóng gói Ứng dụng
- [x] Chạy `cargo check --workspace` và `cargo test --workspace` đảm bảo không lỗi biên dịch và không warnings.
- [x] Chạy thử nghiệm ứng dụng khay hệ thống trên macOS.
- [x] Kiểm tra tương tác:
  - Bấm tray icon -> Popover mở mượt mà.
  - Bấm mở thư mục -> Finder mở đúng thư mục sync.
  - Thử nghiệm ghép cặp và truyền file với CLI dev tool từ Phase 1.
- [x] Cập nhật tài liệu `README.md` hướng dẫn sử dụng ứng dụng đồ hoạ.
