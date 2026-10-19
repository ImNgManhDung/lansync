# LanSync (LAN File Synchronizer)

LanSync is a cross-platform, peer-to-peer (P2P) two-way file synchronizer for local networks (LAN) between **macOS** and **Windows**, written in **Rust**. It functions automatically like a shared folder (similar to Syncthing/Dropbox) but **entirely offline with zero cloud or internet dependency**.

---

## Core Features & Safety Rules

1. **Two-Way Synchronization (2-Way Sync):**
   - Adding or modifying files on one device automatically synchronizes them to paired devices over the local network.
2. **Safety Rule #1 (No Deletion Propagation by Default):**
   - Deleting a file on one machine **never** deletes it on remote machines by default (`never`, `ask`, or `mirror_trash` modes configurable globally or per-peer).
3. **Lossless Conflict Resolution:**
   - When both machines modify the same file while disconnected, reconnecting preserves both copies: the local version and a conflict copy named `filename (conflict-<device-name> YYYY-MM-DD HHMM).ext`.
4. **TLS 1.3 Security + 6-Digit Pairing Confirmation:**
   - End-to-end encrypted mutual TLS (mTLS) with self-signed Ed25519 certificates.
   - When pairing for the first time, both devices display an identical 6-digit confirmation code for verification before allowing any file transfer.
5. **Automatic LAN Discovery via mDNS & Subnet Fallback:**
   - No static IPs needed. Devices on the same Wi-Fi/Ethernet network discover each other automatically via `_lansync._tcp.local.`.
6. **Cross-Platform Path Compatibility:**
   - Unicode NFC normalization (handles macOS NFD decomposed Unicode seamlessly).
   - Case-insensitive conflict and path matching.
   - Ignores OS noise and temporary files: `.DS_Store`, `Thumbs.db`, `~$*`, etc.
7. **Pure Rust Native GUI (`egui` / `eframe`) + Clean Architecture:**
   - Zero WebView / HTML / JS overhead (~15MB RAM footprint, instant startup on macOS & Windows).
   - Native System Tray / Menu Bar integration (`tray-icon` + `muda`) with close-to-tray background operation.

---

## Project Structure

```text
TRANFER_FILE/
├── Cargo.toml
├── docs/superpowers/
│   ├── specs/
│   │   ├── 2026-10-13-lansync-design.md            # Core architecture specification
│   │   └── 2026-10-17-lansync-egui-design.md       # Clean Architecture & Native egui specification
│   └── plans/
│       ├── 2026-10-13-lansync-core.md              # Phase 1: Core sync engine plan
│       ├── 2026-10-14-lansync-hardening-roadmap.md # Hardening roadmap
│       └── 2026-10-17-lansync-egui-native-gui.md   # Phase 2: Native GUI & Clean Architecture plan
├── crates/
│   └── lansync-core/                               # Core synchronization library
│       ├── src/
│       │   ├── paths.rs                            # Unicode NFC normalization & ignore rules
│       │   ├── protocol.rs                         # Bincode framing & message definitions
│       │   ├── fsutil.rs                           # BLAKE3 hashing & mtime helpers
│       │   ├── index.rs                            # SQLite WAL index & change detection
│       │   ├── planner.rs                          # 3-way pure pull-only planner
│       │   ├── identity.rs                         # Ed25519 TLS certificates & Base32 device ID
│       │   ├── pairing.rs                          # 6-digit symmetric pairing code & PeerStore
│       │   ├── transport.rs                        # Mutual TLS 1.3 transport
│       │   ├── transfer.rs                         # Streaming transfer via atomic temp files
│       │   ├── versioning.rs                       # File versioning snapshots (.lansync-versions/)
│       │   ├── watcher.rs                          # Debounced file system watcher
│       │   ├── discovery.rs                        # mDNS LAN discovery service
│       │   ├── config.rs                           # JSON configuration management
│       │   └── engine.rs                           # Unified background sync engine
│       ├── tests/
│       │   └── sync.rs                             # Core integration tests
│       └── examples/
│           └── dev.rs                              # Interactive CLI test tool
└── apps/
    └── lansync-tray/                               # Native Desktop Dashboard & System Tray App (egui/eframe)
        ├── src/
        │   ├── main.rs                             # Composition Root (wires Adapters, UseCases & GUI)
        │   ├── domain/                             # 1. Domain Layer (Entities, Value Objects & Port Traits)
        │   │   ├── models.rs
        │   │   └── ports.rs
        │   ├── application/                        # 2. Application Layer (Use Cases, Commands & State Store)
        │   │   ├── commands.rs
        │   │   ├── state.rs
        │   │   └── use_cases.rs
        │   ├── infrastructure/                     # 3. Infrastructure Layer (Core, OS & System Tray Adapters)
        │   │   ├── core_adapter.rs
        │   │   ├── os_adapter.rs
        │   │   └── tray_adapter.rs
        │   └── presentation/                       # 4. Presentation Layer (egui Dashboard & 5 Tabs)
        │       ├── mod.rs
        │       ├── theme.rs
        │       └── tabs/
        │           ├── sync_tab.rs
        │           ├── devices_tab.rs
        │           ├── versions_tab.rs
        │           ├── activity_tab.rs
        │           └── settings_tab.rs
        └── icons/                                  # Application & Tray icons
```

---

## Getting Started

### 1. Run Automated Test Suite
```bash
cargo test --workspace
```

---

### 2. Run Native Desktop & System Tray App

Launch the pure Rust `egui`/`eframe` desktop application:

```bash
cargo run -p lansync-tray
```

- Opens the **LanSync Desktop Dashboard** (`820x560`) and places an icon in the **macOS Menu Bar** or **Windows System Tray**.
- Clicking the window close button (`X`) hides the window to the tray while background synchronization continues seamlessly.
- Left-clicking the tray icon or using the tray context menu restores the control panel.

---

### 3. CLI Simulation on a Single Machine (Simulating Mac & Windows)

Open two terminal windows to test local synchronization:

#### Terminal 1 (Machine A - "MacBook"):
```bash
cargo run -p lansync-core --example dev -- \
  --name "MacBook-Pro" \
  --dir ./test_folders/mac \
  --data ./test_data/mac \
  --port 42424
```

#### Terminal 2 (Machine B - "Windows-PC"):
```bash
cargo run -p lansync-core --example dev -- \
  --name "Windows-PC" \
  --dir ./test_folders/win \
  --data ./test_data/win \
  --port 42425
```

---

### 4. Running Between Two Real Computers over Wi-Fi / LAN

1. **On macOS:**
   ```bash
   cargo run -p lansync-tray
   ```

2. **On Windows:**
   - Install Rust via [rustup.rs](https://rustup.rs)
   - Run in PowerShell:
   ```powershell
   cargo run -p lansync-tray
   ```

3. Both devices automatically discover each other via mDNS.
4. Verify the 6-digit code and click **Chấp nhận (Approve)**.
5. Any file copied to your sync folder will synchronize automatically.
