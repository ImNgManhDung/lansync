# LanSync (LAN File Synchronizer)

LanSync is a cross-platform, peer-to-peer (P2P) two-way file synchronizer for local networks (LAN) between **macOS** and **Windows**, written in **Rust**. It functions automatically like a shared folder (similar to Syncthing/Dropbox) but **entirely offline with zero cloud or internet dependency**.

---

## Core Features & Safety Rules

1. **Two-Way Synchronization (2-Way Sync):**
   - Adding or modifying files on one device automatically synchronizes them to paired devices over the local network.
2. **Safety Rule #1 (No Deletion Propagation):**
   - Deleting a file on one machine **never** deletes it on remote machines. Deletions remain strictly local to prevent accidental data loss.
3. **Lossless Conflict Resolution:**
   - When both machines modify the same file while disconnected, reconnecting preserves both copies: the local version and a conflict copy named `filename (conflict-<device-name> YYYY-MM-DD HHMM).ext`.
4. **TLS 1.3 Security + 6-Digit Pairing Confirmation:**
   - End-to-end encrypted mutual TLS (mTLS) with self-signed Ed25519 certificates.
   - When pairing for the first time, both devices display an identical 6-digit confirmation code for verification before allowing any file transfer.
5. **Automatic LAN Discovery via mDNS:**
   - No static IPs needed. Devices on the same Wi-Fi/Ethernet network discover each other automatically via `_lansync._tcp.local.`.
6. **Cross-Platform Path Compatibility:**
   - Unicode NFC normalization (handles macOS NFD decomposed Unicode seamlessly).
   - Case-insensitive conflict and path matching.
   - Ignores OS noise and temporary files: `.DS_Store`, `Thumbs.db`, `~$*`, etc.

---

## Project Structure

```text
TRANFER_FILE/
├── Cargo.toml
├── docs/superpowers/
│   ├── specs/2026-10-13-lansync-design.md     # Architecture design specification
│   ├── plans/2026-10-13-lansync-core.md       # Implementation plan: Phase 1 (Core)
│   └── plans/2026-10-14-lansync-tray.md       # Implementation plan: Phase 2 (Tray UI)
├── crates/
│   └── lansync-core/                          # Core synchronization library
│       ├── src/
│       │   ├── paths.rs                       # Unicode NFC normalization & ignore rules
│       │   ├── protocol.rs                    # Bincode framing & message definitions
│       │   ├── fsutil.rs                      # BLAKE3 hashing & mtime helpers
│       │   ├── index.rs                       # SQLite WAL index & change detection
│       │   ├── planner.rs                     # 3-way pure pull-only planner
│       │   ├── identity.rs                    # Ed25519 TLS certificates & Base32 device ID
│       │   ├── pairing.rs                     # 6-digit symmetric pairing code & PeerStore
│       │   ├── transport.rs                   # Mutual TLS 1.3 transport
│       │   ├── transfer.rs                    # Streaming transfer via atomic temp files
│       │   ├── watcher.rs                     # Debounced file system watcher
│       │   ├── discovery.rs                   # mDNS LAN discovery service
│       │   ├── config.rs                      # JSON configuration management
│       │   └── engine.rs                      # Unified background sync engine
│       ├── tests/
│       │   └── sync.rs                        # Core integration tests
│       └── examples/
│           └── dev.rs                         # Interactive CLI test tool
└── apps/
    └── lansync-tray/                          # Native Menu Bar / System Tray Popover App (Tauri v2)
        ├── src/
        │   ├── main.rs                        # App entry point, tray lifecycle & event bridge
        │   ├── state.rs                       # State management & activity logging
        │   ├── commands.rs                    # Tauri IPC invoke commands
        │   └── tray.rs                        # Native system tray and popover window positioning
        ├── ui/                                # Popover UI (HTML/CSS/JS)
        └── icons/                             # Application icons
```

---

## Getting Started

### 1. Run Automated Test Suite
```bash
cargo test --workspace
```
*(All 46 unit and integration tests pass cleanly)*

---

### 2. Run System Tray / Menu Bar App

Launch the native Tauri v2 desktop popover application:

```bash
cargo run -p lansync-tray
```

- The LanSync icon appears in the **macOS Menu Bar** or **Windows Taskbar Tray**.
- Clicking the tray icon opens the control panel popover:
  - Real-time sync status and active file download progress.
  - Quick "Open Folder" shortcut.
  - Automatic peer discovery list.
  - Interactive 6-digit pairing prompt.
  - Settings panel to configure folder and manual connection.

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

#### Pairing and Testing:
1. In **Terminal 1**, initiate connection:
   ```text
   p 127.0.0.1:42425
   ```
2. Both terminals display an identical **6-digit confirmation code** (e.g. `[482190]`).
3. Confirm pairing:
   - In Terminal 1: `y <device_id_of_machine_B>`
   - In Terminal 2: `y <device_id_of_machine_A>`
4. Both machines show `[CONNECTED]` and start syncing:
   - Drop a file into `./test_folders/mac` -> appears in `./test_folders/win`.
   - Delete file in `./test_folders/mac` -> file remains preserved in `./test_folders/win`.

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
4. Verify the 6-digit code and click **Approve**.
5. Any file copied to your sync folder (`~/LanSync` on Mac, `C:\LanSync` on Windows) will synchronize instantly.
