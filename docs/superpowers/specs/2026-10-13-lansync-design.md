# LanSync — Thư mục đồng bộ LAN giữa Mac và Windows (Design Spec)

- **Ngày:** 2026-10-13
- **Trạng thái:** Đã duyệt (2026-10-13), bản sửa 1: bổ sung giao thức ghép cặp, field `Hello` và cách thử lại
- **Thư mục dự án:** `/Users/keith/Documents/TRANFER_FILE`
- **Vị trí:** `docs/superpowers/specs/2026-10-13-lansync-design.md`
- **Implementation plans:** Plan 1 `lansync-core` (lõi + công cụ dev để test tay), Plan 2 `lansync-tray` (viết sau khi Plan 1 xong)

---

## 1. Mục tiêu

Mỗi máy có một thư mục `LanSync`. Bỏ file hoặc sửa file trong thư mục này trên máy nào thì file đó **tự copy sang máy còn lại** qua mạng LAN. Không cần internet, không qua cloud, không phải gõ lệnh.

### Tiêu chí thành công (v1)
1. Copy file 1 GB vào `~/LanSync` trên Mac → file xuất hiện đầy đủ ở `%USERPROFILE%\LanSync` trên Windows, hash khớp.
2. Sửa một file trên Windows → Mac nhận bản mới trong vòng **≤ 5 giây** sau khi lưu xong (với file nhỏ).
3. Hai máy chưa ghép cặp thì **không** trao đổi được dữ liệu với nhau.
4. Cả hai máy cùng sửa một file khi đang mất kết nối → khi kết nối lại, **không mất bản nào**, cả hai bản đều có trên cả hai máy.
5. Tắt một máy, bật lại → tự kết nối lại và đồng bộ phần còn thiếu, không cần người dùng làm gì.

---

## 2. Phạm vi

### Có trong v1
- Đồng bộ 2 chiều cho **thêm file mới** và **sửa file** (bao gồm thư mục con).
- Tự tìm máy trong cùng mạng (mDNS).
- Ghép cặp lần đầu bằng **mã 6 số** hiển thị trên cả hai máy, sau đó chỉ đồng bộ với máy đã ghép.
- Mã hoá đường truyền bằng TLS 1.3 (mutual TLS, chứng chỉ tự ký của từng máy).
- Chạy nền, có icon trên menu bar (Mac) / khay hệ thống (Windows).
- Tự khởi động cùng máy (bật/tắt được).

### Không có trong v1
- **Đồng bộ việc xoá** (theo quyết định của user). Xoá file trên máy nào thì chỉ mất ở máy đó.
- Nhận diện đổi tên/di chuyển (đổi tên = file mới, tên cũ vẫn còn ở máy kia).
- Đồng bộ qua internet / relay, ứng dụng mobile.
- Chọn thư mục con để đồng bộ, lịch sử phiên bản.
- Truyền delta (chỉ phần thay đổi), tiếp tục truyền dở dang (lỗi giữa chừng thì truyền lại cả file).

---

## 3. Các phương án đã cân nhắc

| Phương án | Mô tả | Đánh giá |
|---|---|---|
| **A. Tauri v2 (chỉ tray) + `lansync-core`** ✅ | Lõi đồng bộ là crate Rust thuần. Tauri lo icon khay, cửa sổ ghép cặp nhỏ, autostart, đóng gói `.dmg`/`.msi` | Đóng gói và autostart có sẵn, cửa sổ ghép cặp viết bằng HTML dễ. Phụ thuộc nhiều hơn nhưng chấp nhận được |
| B. Rust thuần (`tray-icon` + `tao`) | Không có webview | Binary nhỏ hơn, nhưng phải tự làm đóng gói, autostart và hộp thoại ghép cặp |
| C. Bọc Syncthing | Gọi Syncthing có sẵn | Không phải app tự viết, trái với mục tiêu |

**Đã chọn: A.**

---

## 4. Kiến trúc

```
TRANFER_FILE/
├── Cargo.toml                  # workspace
├── crates/
│   └── lansync-core/           # toàn bộ logic, không phụ thuộc UI, test được độc lập
│       └── src/
│           ├── lib.rs          # export Engine, EngineEvent, Config
│           ├── config.rs       # đường dẫn thư mục, tên máy, cổng
│           ├── paths.rs        # chuẩn hoá đường dẫn, luật bỏ qua (ignore)
│           ├── index.rs        # quét thư mục, hash blake3, lưu SQLite
│           ├── watcher.rs      # theo dõi thay đổi file (notify, debounce)
│           ├── identity.rs     # chứng chỉ tự ký, device_id
│           ├── pairing.rs      # danh sách máy đã ghép, mã 6 số
│           ├── discovery.rs    # mDNS quảng bá + tìm máy
│           ├── protocol.rs     # định nghĩa message, đóng gói frame
│           ├── transport.rs    # kết nối TCP + TLS 1.3 (mTLS)
│           ├── planner.rs      # so sánh index → quyết định tải/conflict (hàm thuần)
│           ├── transfer.rs     # gửi/nhận file theo chunk, ghi file tạm, rename
│           └── engine.rs       # nối mọi thứ lại, phát EngineEvent cho UI
└── apps/
    └── lansync-tray/           # Tauri v2
        ├── src-tauri/          # khởi tạo Engine, menu khay, lệnh ghép cặp
        └── ui/                 # cửa sổ ghép cặp (HTML/CSS/JS thuần)
```

**Nguyên tắc:** `lansync-core` không biết gì về Tauri. Tray app chỉ gọi `Engine` và nghe `EngineEvent`. Nhờ đó toàn bộ phần đồng bộ được test bằng `cargo test` mà không cần giao diện.

---

## 5. Thành phần chi tiết

### 5.1 Thư mục và dữ liệu nội bộ
| | macOS | Windows |
|---|---|---|
| Thư mục đồng bộ | `~/LanSync` | `%USERPROFILE%\LanSync` |
| Dữ liệu app (index, chứng chỉ, máy đã ghép, log) | `~/Library/Application Support/LanSync/` | `%APPDATA%\LanSync\` |

Dữ liệu app **không** nằm trong thư mục đồng bộ để tránh bị đồng bộ nhầm.

### 5.2 Đường dẫn và luật bỏ qua (`paths.rs`)
- Đường dẫn tương đối lưu dạng `a/b/c.txt`: dấu `/`, Unicode **NFC** (macOS hay trả về NFD, phải chuẩn hoá để hai máy khớp nhau).
- So sánh tên **không phân biệt hoa thường** (APFS và NTFS mặc định đều không phân biệt).
- Bỏ qua: `.DS_Store`, `Thumbs.db`, `desktop.ini`, `~$*` (file khoá của Office), `*.tmp`, `.lansync-tmp-*`, `._*`, symlink.
- Tên không hợp lệ trên Windows (`< > : " | ? *`, tên kết thúc bằng dấu chấm/khoảng trắng, `CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`): **không đồng bộ**, ghi cảnh báo vào log và hiện số lượng file bị bỏ qua trên menu khay.

### 5.3 Index (`index.rs`)
Bảng SQLite (`rusqlite`, bundled):

```sql
CREATE TABLE files (
  path        TEXT PRIMARY KEY,   -- đường dẫn đã chuẩn hoá
  size        INTEGER NOT NULL,
  mtime_ns    INTEGER NOT NULL,
  hash        BLOB,               -- blake3 32 byte; NULL nếu đã xoá cục bộ
  deleted     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE peer_base (
  peer_id     TEXT NOT NULL,
  path        TEXT NOT NULL,
  base_hash   BLOB NOT NULL,      -- hash lúc hai máy khớp nhau lần cuối
  PRIMARY KEY (peer_id, path)
);
```

- Quét lại: chỉ hash lại file có `size` hoặc `mtime` thay đổi.
- File biến mất khỏi đĩa → đặt `deleted = 1` (không xoá dòng). Đây là cách để máy này **không tải lại** file người dùng đã xoá.

### 5.4 Theo dõi thay đổi (`watcher.rs`)
- `notify` + `notify-debouncer-full`, debounce **1 giây**.
- Quét toàn bộ khi khởi động và định kỳ **5 phút/lần** để bù sự kiện bị lỡ.
- File đang được ghi dở (size thay đổi giữa 2 lần đọc cách nhau 500 ms) → chờ lần sau.

### 5.5 Danh tính và ghép cặp (`identity.rs`, `pairing.rs`)
- Lần chạy đầu, mỗi máy sinh chứng chỉ tự ký Ed25519 (`rcgen`).
- `device_id` = 16 ký tự đầu của base32(SHA-256(chứng chỉ DER)). Chứng chỉ được lưu lại nên `device_id` không đổi. Ví dụ: `K7Q2M4XZ9PLA3BCD`.
- `device_id` luôn được **tính từ chứng chỉ TLS của máy kia**, không tin giá trị tự khai trong `Hello`.
- **Ghép cặp (so sánh mã):**
  1. Máy A bấm "Ghép cặp thiết bị mới…", chọn máy B trong danh sách tìm thấy.
  2. A kết nối TLS tới B và gửi `Hello { pairing: true }` (chế độ ghép cặp: tạm chấp nhận chứng chỉ lạ, chỉ dùng cho bước này). B trả lời bằng `Hello` của mình.
  3. Cả hai tính `code = SHA-256(len(certNhỏ) ‖ certNhỏ ‖ certLớn)` (sắp xếp theo byte) → 4 byte đầu `mod 1_000_000` → 6 chữ số. Cùng một kết nối thì hai máy ra cùng mã; nếu có kẻ đứng giữa thì mã sẽ khác.
  4. Cả hai máy hiện mã 6 số và nút "Xác nhận". Người dùng bấm xong thì máy đó gửi `PairDecision { accept }`. Chỉ khi **cả hai** bên `accept = true` thì ghép cặp mới thành công.
  5. Lưu `{device_id, name, cert_fingerprint, last_addr}` vào `peers.json`. Từ đó chỉ chấp nhận đúng chứng chỉ này. Máy A đóng kết nối ghép cặp rồi kết nối lại ngay ở chế độ đồng bộ.
- Mỗi lúc chỉ có một yêu cầu ghép cặp. Yêu cầu thứ hai sẽ bị từ chối.
- Chờ quá 60 giây hoặc một bên bấm huỷ → huỷ ghép cặp.
- Gỡ ghép cặp: menu khay → thiết bị → "Gỡ ghép cặp".

### 5.6 Tìm máy (`discovery.rs`)
- `mdns-sd`, service `_lansync._tcp.local.`, TXT: `id=<device_id>`, `name=<tên máy>`, `v=1`.
- Cổng TCP mặc định **47800**. Nếu cổng đã bị chiếm thì dùng cổng ngẫu nhiên và quảng bá cổng đó.
- Fallback khi mDNS bị chặn: người dùng nhập IP thủ công trong cửa sổ ghép cặp (IP được lưu vào `peers.json`).

### 5.7 Giao thức (`protocol.rs`, `transport.rs`)
- TCP + `tokio-rustls`, TLS 1.3, xác thực cả hai chiều (mTLS).
- Frame: `u32 big-endian độ dài` + payload `bincode` (serde). Frame điều khiển tối đa 16 MB.

```rust
enum Message {
    Hello { device_id: String, name: String, proto: u16, pairing: bool, listen_port: u16 }, // proto = 1
    PairDecision { accept: bool },                                // chỉ dùng trong kết nối ghép cặp
    Index { entries: Vec<Entry> },                                // gửi toàn bộ khi mới kết nối
    IndexUpdate { entries: Vec<Entry> },                          // gửi khi có thay đổi
    GetFile { path: String, hash: [u8; 32] },
    FileHeader { path: String, size: u64, hash: [u8; 32], mtime_ns: i64 },
    FileChunk { data: Vec<u8> },                                  // tối đa 256 KiB
    FileEnd,
    FileUnavailable { path: String },                             // file đã đổi/xoá ở phía gửi
    Ping, Pong,
}
struct Entry { path: String, size: u64, mtime_ns: i64, hash: Option<[u8; 32]> } // None = đã xoá cục bộ
```

- Bắt đầu kết nối: cả hai bên gửi `Hello` rồi đọc `Hello` của bên kia. `listen_port` cho máy kia biết cổng để kết nối lại (lưu vào `last_addr`).
- Index lớn được chia thành nhiều frame: frame đầu là `Index`, các frame sau là `IndexUpdate`, mỗi frame tối đa 5.000 entry.
- Ping mỗi 15 giây, mất 45 giây không có phản hồi → coi như mất kết nối, thử lại sau 5 → 10 → 30 giây (tối đa 30 giây).
- Nếu cả hai máy cùng kết nối tới nhau một lúc: giữ kết nối do máy có `device_id` nhỏ hơn khởi tạo.

### 5.8 Lập kế hoạch đồng bộ (`planner.rs`) — hàm thuần, test kỹ nhất
**Mô hình chỉ kéo về (pull-only):** mỗi máy chỉ tự quyết định **tải gì về**, không bao giờ đẩy file sang máy kia. Hai máy chạy cùng một thuật toán đối xứng.

Với mỗi `path`, gọi `L` = bản ở máy mình, `R` = bản ở máy kia, `B` = `base_hash` của cặp máy:

| Tình huống | Hành động |
|---|---|
| `R` không có hoặc `R` đã xoá | Không làm gì (không đồng bộ việc xoá) |
| `L` không có, `R` có | **Tải về** |
| `L` đã xoá cục bộ, `R.hash == B` | Không làm gì (tôn trọng việc người dùng đã xoá) |
| `L` đã xoá cục bộ, `R.hash != B` | **Tải về** (máy kia đã sửa nên coi là nội dung mới) |
| `L.hash == R.hash` | Cập nhật `B = L.hash` |
| `L.hash == B`, `R.hash != B` | **Tải về** (chỉ máy kia sửa) |
| `R.hash == B`, `L.hash != B` | Không làm gì (chỉ mình sửa, máy kia sẽ tự kéo) |
| `L`, `R`, `B` đều khác nhau (hoặc chưa có `B`) | **Xung đột** → xem dưới |

**Xử lý xung đột (tất định, không mất dữ liệu):**
- Máy có `device_id` **nhỏ hơn** là bên "thắng", giữ nguyên tên file.
- Máy "thua" đổi tên bản của mình thành `tên (conflict-<tên máy> 2026-10-13 1705).ext`, sau đó tải bản của máy thắng về đúng tên gốc.
- File conflict là file mới, nên lần đồng bộ sau sẽ tự sang máy thắng → cuối cùng **cả hai máy có cả hai bản**.

### 5.9 Truyền file (`transfer.rs`)
1. Ghi vào `<thư mục đích>/.lansync-tmp-<random>` (file này bị luật bỏ qua loại ra).
2. Vừa nhận vừa tính blake3. Hash không khớp `FileHeader.hash` → xoá file tạm, ghi log lỗi, thử lại tối đa 3 lần.
3. Ngay trước khi rename: kiểm tra file đích không bị đổi kể từ lúc lập kế hoạch (so `L.hash` với hash hiện tại). Nếu đã bị đổi → huỷ, lập kế hoạch lại.
4. Đặt `mtime` theo máy gửi → rename đè (atomic) → cập nhật `files` và `peer_base`.
5. Mỗi lần chỉ truyền một file trên một kết nối (đơn giản cho v1). Bên gửi đọc file theo luồng, không nạp hết vào RAM.
6. Bên gửi: nếu hash file hiện tại khác hash được yêu cầu → trả `FileUnavailable`, bên nhận chờ `IndexUpdate` mới.

### 5.10 Engine và sự kiện (`engine.rs`)
```rust
pub struct Engine;  // Engine::start(Config) -> Result<EngineHandle>
pub enum EngineEvent {
    Status(SyncStatus),                    // Idle | Syncing { pending: usize } | Offline
    PeerOnline { id: String, name: String },
    PeerOffline { id: String },
    PairingRequest { peer_name: String, code: String }, // máy kia muốn ghép
    PairingResult { peer_id: String, ok: bool },
    FileSynced { path: String },
    ConflictCreated { path: String },
    Skipped { path: String, reason: String },
    Error(String),
}
// EngineHandle: subscribe(), discovered_peers(), pair(addr), confirm_pairing(bool), unpair(id), shutdown()
```

### 5.11 Ứng dụng khay (`apps/lansync-tray`)
Menu khay:
- Dòng trạng thái: `✓ Đã đồng bộ` / `⟳ Đang đồng bộ 3 file` / `○ Chưa kết nối máy nào`
- `Mở thư mục LanSync`
- `Thiết bị ▸` → danh sách máy đã ghép (● online / ○ offline), mỗi máy có mục `Gỡ ghép cặp`
- `Ghép cặp thiết bị mới…` → mở cửa sổ nhỏ (420×520): danh sách máy tìm thấy, ô nhập IP, màn hình mã 6 số + nút Xác nhận/Huỷ
- `File bị bỏ qua (n)` (chỉ hiện khi n > 0) → mở file log
- `☐ Khởi động cùng máy` (`tauri-plugin-autostart`)
- `Thoát`

Khi máy kia gửi yêu cầu ghép cặp → cửa sổ ghép cặp tự bật lên với mã 6 số.
Không có Dock icon trên Mac (`ActivationPolicy::Accessory`).

### 5.12 Lưu ý theo nền tảng
- **macOS 15+:** lần đầu chạy sẽ hỏi quyền "Local Network". Phải khai báo `NSLocalNetworkUsageDescription` và `NSBonjourServices = [_lansync._tcp]` trong `Info.plist`.
- **Windows:** lần đầu lắng nghe cổng, Windows Firewall sẽ hỏi. Installer `.msi` thêm luật firewall cho cổng 47800 (mạng Private).
- Mạng Wi-Fi bật "client isolation" (thường gặp ở mạng công cộng/khách) thì hai máy không thấy nhau. Trường hợp này ghi trong README, không xử lý.

---

## 6. Xử lý lỗi

| Lỗi | Cách xử lý |
|---|---|
| Mất mạng giữa lúc truyền | Xoá file tạm, kết nối lại có backoff, lập kế hoạch lại từ đầu |
| Hash không khớp | Thử lại ngay, tối đa 3 lần. Sau đó bỏ qua **phiên bản đó** (cặp path + hash) cho tới khi app khởi động lại hoặc máy kia có bản mới, kèm `EngineEvent::Error` |
| Ổ đĩa đầy / lỗi ghi khác | Huỷ file tạm, phát `Error`. Mỗi phiên đồng bộ tự chạy lại sau **5 phút**, hoặc sớm hơn nếu có thay đổi |
| File đang bị khoá (Windows, Office đang mở) | Không rename được → giữ file tạm, thử lại sau 10 giây, tối đa 30 lần |
| Chứng chỉ lạ (không có trong danh sách đã ghép) | Từ chối kết nối, ghi log |
| `proto` không khớp | Từ chối, hiện "Phiên bản LanSync trên <máy> không tương thích" |
| Thư mục `LanSync` bị xoá | Tạo lại thư mục rỗng; vì `deleted=1` nên không tự tải lại hết (giống trường hợp người dùng xoá file) |

Log: `tracing` + `tracing-appender`, xoay vòng mỗi ngày, giữ 7 file, nằm trong thư mục dữ liệu app.

---

## 7. Kiểm thử

1. **Unit test (`lansync-core`)**
   - `paths`: NFC/NFD, phân biệt hoa thường, luật bỏ qua, tên không hợp lệ trên Windows.
   - `planner`: mỗi dòng trong bảng 5.8 có ít nhất 1 test, cộng thêm test đặt tên file xung đột.
   - `protocol`: đóng gói/giải mã frame round-trip, từ chối frame quá lớn.
   - `index`: quét, phát hiện sửa, đánh dấu đã xoá.
   - `pairing`: hai bên tính ra cùng một mã, đổi chứng chỉ thì mã khác.
2. **Integration test**: chạy 2 `Engine` trong cùng process trên `127.0.0.1` với 2 thư mục tạm (`tempfile`), tắt mDNS và kết nối trực tiếp. Kịch bản: thêm, sửa, xung đột, xoá không lan sang máy kia, mất kết nối rồi kết nối lại, file 200 MB.
3. **Test thủ công Mac ⇄ Windows**: danh sách kiểm tra nằm trong implementation plan.

---

## 8. Ràng buộc chung

- Rust stable **1.98**, edition **2024**.
- Tauri **v2**.
- Nền tảng: **macOS 13+** (Apple Silicon + Intel), **Windows 10/11 x64**.
- Thiết kế cho **2 máy**. Giao thức không cấm nhiều hơn, nhưng v1 chỉ test với 2 máy.
- Thư viện chính: `tokio`, `tokio-rustls`/`rustls`, `rcgen`, `mdns-sd`, `notify`, `notify-debouncer-full`, `rusqlite` (bundled), `blake3`, `serde`, `bincode`, `unicode-normalization`, `directories`, `tracing`, `tauri`, `tauri-plugin-autostart`.
- Không dùng dịch vụ cloud, không gửi dữ liệu ra ngoài mạng LAN.
