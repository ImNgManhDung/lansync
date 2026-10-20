mod config_tests {
    use lansync_core::config::{Config, DeleteMode};
    use tempfile::tempdir;

    #[test]
    fn test_config_save_load() {
        let dir = tempdir().unwrap();
        let cfg_path = dir.path().join("config.json");
        let sync_dir = dir.path().join("LanSync");

        let cfg = Config::load_or_default(&cfg_path, sync_dir.clone()).unwrap();
        assert_eq!(cfg.listen_port, 42424);
        assert_eq!(cfg.sync_dir, sync_dir);
        assert_eq!(cfg.delete_mode, DeleteMode::Never);

        let mut cfg2 = cfg.clone();
        cfg2.listen_port = 50000;
        cfg2.delete_mode = DeleteMode::MirrorTrash;
        cfg2.save(&cfg_path).unwrap();

        let loaded = Config::load_or_default(&cfg_path, sync_dir).unwrap();
        assert_eq!(loaded.listen_port, 50000);
        assert_eq!(loaded.delete_mode, DeleteMode::MirrorTrash);
    }
}

mod discovery_tests {
    use lansync_core::discovery::{address_priority, is_lan_routable_address, DiscoveryService};

    #[tokio::test]
    async fn test_discovery_service_lifecycle() {
        let mut service = DiscoveryService::new("DEVTEST123456789").unwrap();
        service.register("TestDevice", 42424).unwrap();
        let _rx = service.browse().unwrap();
        service.shutdown().unwrap();
    }

    #[test]
    fn test_lan_routable_address_filtering_and_priority() {
        use std::net::{IpAddr, Ipv4Addr};

        let loopback: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
        let docker: IpAddr = Ipv4Addr::new(172, 17, 0, 1).into();
        let tailscale: IpAddr = Ipv4Addr::new(100, 80, 1, 2).into();
        let home_lan: IpAddr = Ipv4Addr::new(192, 168, 1, 50).into();
        let office_lan: IpAddr = Ipv4Addr::new(10, 0, 0, 20).into();

        assert!(!is_lan_routable_address(&loopback));
        assert!(!is_lan_routable_address(&docker));
        assert!(!is_lan_routable_address(&tailscale));
        assert!(is_lan_routable_address(&home_lan));
        assert!(is_lan_routable_address(&office_lan));

        let mut ips = vec![office_lan, home_lan];
        ips.sort_by_key(address_priority);
        assert_eq!(ips[0], home_lan);
        assert_eq!(ips[1], office_lan);
    }
}

mod engine_tests {
    use lansync_core::config::Config;
    use lansync_core::engine::{is_active_dialer, next_delay, PeerStatus, SyncEngine, SyncEvent};
    use std::time::Duration;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_engine_start_and_shutdown() {
        let dir = tempdir().unwrap();
        let sync_dir = dir.path().join("sync");
        let data_dir = dir.path().join("data");
        let cfg = Config::new(sync_dir, 0, "TestDev".into());
        let engine = SyncEngine::start(cfg, data_dir.clone()).await.unwrap();
        assert!(!engine.local_device_id().is_empty());
        engine.shutdown().await.unwrap();
        assert!(
            data_dir.join(".clean_shutdown").exists(),
            "Clean shutdown flag must be saved upon exit"
        );
    }

    #[tokio::test]
    async fn test_engine_pause_and_bandwidth_limit() {
        let dir = tempdir().unwrap();
        let sync_dir = dir.path().join("sync");
        let data_dir = dir.path().join("data");
        let cfg = Config::new(sync_dir, 0, "TestDev".into());
        let engine = SyncEngine::start(cfg, data_dir).await.unwrap();

        assert!(!engine.is_paused());
        engine.set_paused(true).await;
        assert!(engine.is_paused());
        engine.set_paused(false).await;
        assert!(!engine.is_paused());

        assert_eq!(engine.get_bandwidth_limit_kbps(), 0);
        engine.set_bandwidth_limit_kbps(2048);
        assert_eq!(engine.get_bandwidth_limit_kbps(), 2048);

        engine.shutdown().await.unwrap();
    }

    #[test]
    fn test_is_active_dialer_tie_breaker() {
        let dev_a = "AAAA1111BBBB2222";
        let dev_b = "ZZZZ9999YYYY8888";
        assert!(
            is_active_dialer(dev_a, dev_b),
            "Device A (smaller) must be active dialer"
        );
        assert!(
            !is_active_dialer(dev_b, dev_a),
            "Device B (larger) must NOT be active dialer"
        );
    }

    #[test]
    fn test_next_delay_backoff_and_jitter() {
        for _ in 0..10 {
            let d0 = next_delay(0);
            assert!(d0 >= Duration::from_millis(800) && d0 <= Duration::from_millis(1200));

            let d1 = next_delay(1);
            assert!(d1 >= Duration::from_millis(1600) && d1 <= Duration::from_millis(2400));

            let d6 = next_delay(6);
            assert!(d6 >= Duration::from_secs(48) && d6 <= Duration::from_secs(72));

            let d10 = next_delay(10);
            // Cap at 60s max base
            assert!(d10 >= Duration::from_secs(48) && d10 <= Duration::from_secs(72));
        }
    }

    #[tokio::test]
    async fn test_peer_status_transitions_and_events() {
        let dir = tempdir().unwrap();
        let sync_dir = dir.path().join("sync");
        let data_dir = dir.path().join("data");
        let cfg = Config::new(sync_dir, 0, "TestDev".into());
        let engine = SyncEngine::start(cfg, data_dir).await.unwrap();

        let mut rx = engine.subscribe_events();

        assert_eq!(engine.get_peer_status("PEER123").await, PeerStatus::Offline);

        engine
            .set_peer_status("PEER123", PeerStatus::Connecting)
            .await;
        assert_eq!(
            engine.get_peer_status("PEER123").await,
            PeerStatus::Connecting
        );

        let ev = rx.recv().await.unwrap();
        if let SyncEvent::PeerStatusChanged { peer_id, status } = ev {
            assert_eq!(peer_id, "PEER123");
            assert_eq!(status, PeerStatus::Connecting);
        } else {
            panic!("Expected PeerStatusChanged event");
        }

        engine
            .set_peer_status("PEER123", PeerStatus::Connected)
            .await;
        assert_eq!(
            engine.get_peer_status("PEER123").await,
            PeerStatus::Connected
        );

        let ev = rx.recv().await.unwrap();
        if let SyncEvent::PeerStatusChanged { peer_id, status } = ev {
            assert_eq!(peer_id, "PEER123");
            assert_eq!(status, PeerStatus::Connected);
        } else {
            panic!("Expected PeerStatusChanged event");
        }

        engine.shutdown().await.unwrap();
    }
}

mod fsutil_tests {
    use filetime::FileTime;
    use lansync_core::fsutil::*;
    use std::time::Duration;

    #[test]
    fn hash_matches_blake3() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"hello world").unwrap();
        assert_eq!(
            hash_file(&p).unwrap(),
            *blake3::hash(b"hello world").as_bytes()
        );
    }

    #[test]
    fn hash_file_stable_returns_hash_when_settled() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("stable.txt");
        std::fs::write(&p, b"settled content").unwrap();
        let h = hash_file_stable(&p).unwrap();
        assert_eq!(h, Some(*blake3::hash(b"settled content").as_bytes()));
    }

    #[tokio::test]
    async fn open_with_retry_succeeds_on_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("retry.txt");
        std::fs::write(&p, b"test content").unwrap();
        let res = open_with_retry(&p, Duration::from_secs(1)).await;
        assert!(res.is_ok());
    }

    #[test]
    fn mtime_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"x").unwrap();
        let ns: i64 = 1_700_000_000_123_456_700; // multiple of 100ns to match NTFS resolution
        filetime::set_file_mtime(&p, filetime_from_ns(ns)).unwrap();
        assert_eq!(mtime_ns(&std::fs::metadata(&p).unwrap()), ns);
    }

    #[test]
    fn test_available_disk_space() {
        let dir = tempfile::tempdir().unwrap();
        let space = available_disk_space(dir.path()).unwrap();
        assert!(space > 0, "Available space should be greater than 0");
    }

    #[test]
    fn test_cleanup_orphaned_temps() {
        let dir = tempfile::tempdir().unwrap();
        let old_tmp = dir.path().join(".lansync-tmp-old123");
        let new_tmp = dir.path().join(".lansync-tmp-new456");
        let regular = dir.path().join("keep.txt");

        std::fs::write(&old_tmp, b"old").unwrap();
        std::fs::write(&new_tmp, b"new").unwrap();
        std::fs::write(&regular, b"regular").unwrap();

        let old_time =
            FileTime::from_unix_time(filetime::FileTime::now().unix_seconds() - 48 * 3600, 0);
        filetime::set_file_mtime(&old_tmp, old_time).unwrap();

        let removed = cleanup_orphaned_temps(dir.path(), Duration::from_secs(24 * 3600)).unwrap();
        assert_eq!(removed, 1);
        assert!(!old_tmp.exists());
        assert!(new_tmp.exists());
        assert!(regular.exists());
    }
}

mod identity_tests {
    use lansync_core::identity::*;

    #[test]
    fn device_id_is_16_base32_chars() {
        let id = Identity::generate().unwrap();
        assert_eq!(id.device_id.len(), 16);
        assert!(id
            .device_id
            .chars()
            .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
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
        assert_eq!(
            fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

mod index_tests {
    use lansync_core::index::{Index, ScanReport};
    use lansync_core::protocol::Entry;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PathBuf, Index) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("sync");
        fs::create_dir_all(&root).unwrap();
        let idx = Index::open(&t.path().join("index.db"), &root, "dev_test").unwrap();
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
        // second scan does not re-report
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
        let e = Entry::simple("x.txt", 5, 9, Some([3; 32]));
        idx.record_file(&e).unwrap();
        assert_eq!(idx.get("x.txt").unwrap(), Some(e));
        idx.set_base("PEER", "x.txt", &[1; 32]).unwrap();
        idx.set_base("PEER", "x.txt", &[2; 32]).unwrap();
        assert_eq!(idx.bases("PEER").unwrap().get("x.txt"), Some(&[2; 32]));
        assert!(idx.bases("OTHER").unwrap().is_empty());
    }

    #[test]
    fn version_vector_increments_and_persists() {
        let (t, root, mut idx) = setup();
        fs::write(root.join("vv_test.txt"), b"v1").unwrap();
        idx.scan().unwrap();
        let e1 = idx.get("vv_test.txt").unwrap().unwrap();
        assert_eq!(e1.vv.get("dev_test"), 1);

        // Edit file
        fs::write(root.join("vv_test.txt"), b"v2_content").unwrap();
        idx.scan().unwrap();
        let e2 = idx.get("vv_test.txt").unwrap().unwrap();
        assert_eq!(e2.vv.get("dev_test"), 2);

        // Delete file
        fs::remove_file(root.join("vv_test.txt")).unwrap();
        idx.scan().unwrap();
        let e3 = idx.get("vv_test.txt").unwrap().unwrap();
        assert_eq!(e3.vv.get("dev_test"), 3);
        assert_eq!(e3.hash, None);

        // Reopen DB and verify persistence
        let idx_reopened = Index::open(&t.path().join("index.db"), &root, "dev_test").unwrap();
        let e_persisted = idx_reopened.get("vv_test.txt").unwrap().unwrap();
        assert_eq!(e_persisted.vv.get("dev_test"), 3);
    }

    #[test]
    fn tombstone_recorded_on_delete_and_cleared_on_recreate() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("test.pdf"), b"pdf-content").unwrap();
        idx.scan().unwrap();
        let expected_hash = *blake3::hash(b"pdf-content").as_bytes();
        assert!(idx.tombstones().unwrap().is_empty());

        // Delete file
        fs::remove_file(root.join("test.pdf")).unwrap();
        idx.scan().unwrap();

        // Tombstone must be recorded with original file's hash!
        let tombstones = idx.tombstones().unwrap();
        assert_eq!(tombstones.get("test.pdf"), Some(&expected_hash));

        // Re-creating file clears the tombstone
        fs::write(root.join("test.pdf"), b"new-pdf-content").unwrap();
        idx.scan().unwrap();
        assert!(idx.tombstones().unwrap().is_empty());
    }

    #[test]
    fn unmounted_root_prevents_mass_deletion() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("important1.doc"), b"data1").unwrap();
        fs::write(root.join("important2.doc"), b"data2").unwrap();
        idx.scan().unwrap();
        assert_eq!(
            idx.entries()
                .unwrap()
                .iter()
                .filter(|e| e.hash.is_some())
                .count(),
            2
        );

        // Simulate external drive unmount by removing root directory or emptying it without marker
        fs::remove_file(root.join(".lansync-root")).unwrap();
        fs::remove_file(root.join("important1.doc")).unwrap();
        fs::remove_file(root.join("important2.doc")).unwrap();

        let report = idx.scan().unwrap();
        // Mass deletion must be halted!
        assert!(
            report.changed.is_empty(),
            "Unmounted root must not report deletions!"
        );
        assert_eq!(
            idx.entries()
                .unwrap()
                .iter()
                .filter(|e| e.hash.is_some())
                .count(),
            2
        );
    }

    #[test]
    fn test_custom_lansyncignore_is_respected() {
        let (_t, root, mut idx) = setup();
        fs::write(
            root.join(".lansyncignore"),
            b"*.log\nbuild/\nsecret_key.pem\n",
        )
        .unwrap();
        fs::write(root.join("app.log"), b"log").unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::write(root.join("build/output.bin"), b"binary").unwrap();
        fs::write(root.join("secret_key.pem"), b"secret").unwrap();
        fs::write(root.join("allowed.txt"), b"allowed").unwrap();

        let report = idx.scan().unwrap();
        let changed_paths = paths(&report);

        assert_eq!(changed_paths, vec!["allowed.txt"]);
    }
}

mod pairing_tests {
    use lansync_core::config::DeleteMode;
    use lansync_core::pairing::*;
    use std::time::{Duration, Instant};

    #[test]
    fn code_is_symmetric_six_digits_and_cert_dependent() {
        let c1 = pairing_code(b"cert-mac", b"cert-win");
        assert_eq!(c1, pairing_code(b"cert-win", b"cert-mac"));
        assert_eq!(c1.len(), 6);
        assert!(c1.chars().all(|c| c.is_ascii_digit()));
        assert_ne!(c1, pairing_code(b"cert-mac", b"cert-attacker"));
    }

    fn peer(id: &str, fp: &str) -> Peer {
        Peer {
            device_id: id.into(),
            name: format!("Device {id}"),
            cert_fingerprint: fp.into(),
            last_addr: None,
            delete_mode: None,
        }
    }

    #[test]
    fn store_persists_and_replaces_same_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let mut s = PeerStore::load(&path).unwrap();
        assert!(s.list().is_empty());
        s.add(peer("A", "fp1"));
        s.add(peer("B", "fp2"));
        s.add(peer("A", "fp3")); // Re-pair -> replace old entry
        s.set_addr("B", "192.168.1.5:47800");
        s.save().unwrap();

        let s2 = PeerStore::load(&path).unwrap();
        assert_eq!(s2.list().len(), 2);
        assert_eq!(s2.find_by_fingerprint("fp3").unwrap().device_id, "A");
        assert!(s2.find_by_fingerprint("fp1").is_none());
        assert_eq!(
            s2.find_by_fingerprint("fp2").unwrap().last_addr.as_deref(),
            Some("192.168.1.5:47800")
        );
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

    #[test]
    fn test_peer_delete_mode_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let mut s = PeerStore::load(&path).unwrap();
        s.add(peer("dev1", "fp1"));
        assert_eq!(s.list()[0].delete_mode, None);

        assert!(s.set_delete_mode("dev1", Some(DeleteMode::MirrorTrash)));
        assert_eq!(s.list()[0].delete_mode, Some(DeleteMode::MirrorTrash));
        s.save().unwrap();

        let loaded = PeerStore::load(&path).unwrap();
        assert_eq!(loaded.list()[0].delete_mode, Some(DeleteMode::MirrorTrash));
    }

    #[test]
    fn pairing_limiter_throttles_brute_force_and_locks_out() {
        let mut limiter = PairingLimiter::new(3, Duration::from_secs(90), Duration::from_secs(300));
        let now = Instant::now();
        let peer_ip = "192.168.1.100";

        // Initial attempts allowed
        assert!(limiter.can_attempt(peer_ip, now).is_ok());

        // First failure
        limiter.record_failure(peer_ip, now);
        assert!(limiter.can_attempt(peer_ip, now).is_ok());

        // Second failure
        limiter.record_failure(peer_ip, now);
        assert!(limiter.can_attempt(peer_ip, now).is_ok());

        // Third failure: triggers lockout
        limiter.record_failure(peer_ip, now);
        let res = limiter.can_attempt(peer_ip, now);
        assert!(matches!(res, Err(PairingLimiterError::LockedOut { .. })));

        // Advance time before lockout ends (200s < 300s) -> still locked out
        let later_still_locked = now + Duration::from_secs(200);
        assert!(matches!(
            limiter.can_attempt(peer_ip, later_still_locked),
            Err(PairingLimiterError::LockedOut { .. })
        ));

        // Advance time past lockout (301s >= 300s) -> allowed again
        let later_unlocked = now + Duration::from_secs(301);
        assert!(limiter.can_attempt(peer_ip, later_unlocked).is_ok());

        // Success clears all failure counters
        limiter.record_success(peer_ip);
        assert!(limiter.can_attempt(peer_ip, later_unlocked).is_ok());
    }

    #[test]
    fn pairing_code_ttl_expiration() {
        let limiter = PairingLimiter::default_policy();
        let created_at = Instant::now();

        assert!(!limiter.is_expired(created_at, created_at + Duration::from_secs(30)));
        assert!(!limiter.is_expired(created_at, created_at + Duration::from_secs(90)));
        assert!(limiter.is_expired(created_at, created_at + Duration::from_secs(91)));
    }
}

mod paths_tests {
    use lansync_core::paths::*;
    use std::path::Path;

    #[test]
    fn normalize_rel_uses_slash_and_nfc() {
        let root = Path::new("/tmp/root");
        // Unicode string in NFD form (e + combining circumflex + combining acute) commonly returned by macOS
        let full = root.join("folder").join("Tie\u{0302}\u{0301}ng.txt");
        assert_eq!(
            normalize_rel(root, &full).as_deref(),
            Some("folder/Ti\u{1EBF}ng.txt")
        );
    }

    #[test]
    fn normalize_rel_rejects_outside_root_and_root_itself() {
        let root = Path::new("/tmp/root");
        assert_eq!(normalize_rel(root, Path::new("/tmp/other/a.txt")), None);
        assert_eq!(normalize_rel(root, root), None);
    }

    #[test]
    fn test_normalize_rel_windows_unc_and_case() {
        let root_unc = Path::new(r"\\?\C:\Users\Admin\LanSync");
        let full_plain = Path::new(r"C:\Users\Admin\LanSync\sub\test.txt");
        assert_eq!(
            normalize_rel(root_unc, full_plain).as_deref(),
            Some("sub/test.txt")
        );

        let root_lower = Path::new(r"c:\users\admin\lansync");
        let full_upper = Path::new(r"C:\Users\Admin\LanSync\test.txt");
        assert_eq!(
            normalize_rel(root_lower, full_upper).as_deref(),
            Some("test.txt")
        );
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
        for p in [
            ".DS_Store",
            "a/.DS_Store",
            "Thumbs.db",
            "desktop.ini",
            "~$report.docx",
            "x.tmp",
            "X.TMP",
            "sub/.lansync-tmp-abc",
            "._file",
        ] {
            assert!(is_ignored(p), "should ignore {p}");
        }
        for p in ["a.txt", "tmp/a.txt", "report.docx", ".env"] {
            assert!(!is_ignored(p), "should not ignore {p}");
        }
    }

    #[test]
    fn windows_invalid_names() {
        for p in [
            "a:b.txt",
            "sub/what?.txt",
            "CON",
            "con.txt",
            "COM1.log",
            "lpt9",
            "ends-with-dot.",
            "space ",
        ] {
            assert!(
                windows_invalid_reason(p).is_some(),
                "{p} should be invalid on Windows"
            );
        }
        for p in ["COM10.txt", "console.txt", "a/b.txt", "Report.docx"] {
            assert!(
                windows_invalid_reason(p).is_none(),
                "{p} should be valid on Windows"
            );
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
        assert_eq!(
            validate_rel_path("docs\\note.md").as_deref(),
            Some("docs/note.md")
        );
    }

    #[test]
    fn test_sanitize_for_windows() {
        assert_eq!(sanitize_for_windows("a:b.txt"), "a_b.txt");
        assert_eq!(sanitize_for_windows("sub/file?.pdf"), "sub/file_.pdf");
        assert_eq!(sanitize_for_windows("CON.txt"), "CON_renamed.txt");
        assert_eq!(
            sanitize_for_windows("folder/ends-with-dot."),
            "folder/ends-with-dot"
        );
        assert_eq!(sanitize_for_windows("AUX"), "AUX_renamed");
        assert_eq!(sanitize_for_windows("COM1.log"), "COM1_renamed.log");
    }

    #[test]
    fn test_resolve_case_collision() {
        assert_eq!(
            resolve_case_collision("Report.docx"),
            "Report (case-conflict).docx"
        );
        assert_eq!(
            resolve_case_collision("sub/file.txt"),
            "sub/file (case-conflict).txt"
        );
        assert_eq!(
            resolve_case_collision("Makefile"),
            "Makefile (case-conflict)"
        );
    }
}

mod planner_tests {
    use lansync_core::config::DeleteMode;
    use lansync_core::planner::*;
    use lansync_core::protocol::{Entry, Hash};
    use lansync_core::version_vector::VersionVector;
    use std::collections::HashMap;

    fn h(n: u8) -> Hash {
        [n; 32]
    }
    fn e(path: &str, hash: Option<u8>) -> Entry {
        Entry::simple(path, 1, 1, hash.map(h))
    }
    fn run(local: &[Entry], remote: &[Entry], base: &[(&str, u8)], win: bool) -> Vec<Action> {
        let base: HashMap<String, Hash> =
            base.iter().map(|(p, n)| (p.to_string(), h(*n))).collect();
        let tombstones = HashMap::new();
        plan(&PlanInput {
            local,
            remote,
            base: &base,
            tombstones: &tombstones,
            i_win_conflicts: win,
            my_name: "Mac",
            stamp: "2026-10-13 1705",
            delete_mode: DeleteMode::Never,
        })
    }

    fn run_with_tombstones(
        local: &[Entry],
        remote: &[Entry],
        base: &[(&str, u8)],
        tombstones: &[(&str, u8)],
        win: bool,
    ) -> Vec<Action> {
        let base: HashMap<String, Hash> =
            base.iter().map(|(p, n)| (p.to_string(), h(*n))).collect();
        let tombstones: HashMap<String, Hash> = tombstones
            .iter()
            .map(|(p, n)| (p.to_string(), h(*n)))
            .collect();
        plan(&PlanInput {
            local,
            remote,
            base: &base,
            tombstones: &tombstones,
            i_win_conflicts: win,
            my_name: "Mac",
            stamp: "2026-10-13 1705",
            delete_mode: DeleteMode::Never,
        })
    }

    fn run_with_mode(
        local: &[Entry],
        remote: &[Entry],
        base: &[(&str, u8)],
        mode: DeleteMode,
    ) -> Vec<Action> {
        let base: HashMap<String, Hash> =
            base.iter().map(|(p, n)| (p.to_string(), h(*n))).collect();
        let tombstones = HashMap::new();
        plan(&PlanInput {
            local,
            remote,
            base: &base,
            tombstones: &tombstones,
            i_win_conflicts: false,
            my_name: "Mac",
            stamp: "2026-10-13 1705",
            delete_mode: mode,
        })
    }

    #[test]
    fn tombstone_prevents_deleted_file_resurrection_unless_modified_remotely() {
        let actions = run_with_tombstones(&[], &[e("a.pdf", Some(1))], &[], &[("a.pdf", 1)], false);
        assert!(
            actions.is_empty(),
            "Tombstone matching remote hash must not download!"
        );

        let actions_modified =
            run_with_tombstones(&[], &[e("a.pdf", Some(2))], &[], &[("a.pdf", 1)], false);
        assert_eq!(
            actions_modified,
            vec![Action::Download {
                local_path: "a.pdf".into(),
                remote_path: "a.pdf".into(),
                hash: h(2),
                expected_local: None
            }]
        );
    }

    #[test]
    fn remote_deleted_does_nothing() {
        assert!(run(&[e("a", Some(1))], &[e("a", None)], &[("a", 1)], false).is_empty());
    }

    #[test]
    fn missing_locally_downloads() {
        assert_eq!(
            run(&[], &[e("a", Some(1))], &[], false),
            vec![Action::Download {
                local_path: "a".into(),
                remote_path: "a".into(),
                hash: h(1),
                expected_local: None
            }]
        );
    }

    #[test]
    fn locally_deleted_and_remote_unchanged_is_respected() {
        assert!(run(&[e("a", None)], &[e("a", Some(1))], &[("a", 1)], false).is_empty());
    }

    #[test]
    fn locally_deleted_but_remote_changed_downloads() {
        assert_eq!(
            run(&[e("a", None)], &[e("a", Some(2))], &[("a", 1)], false),
            vec![Action::Download {
                local_path: "a".into(),
                remote_path: "a".into(),
                hash: h(2),
                expected_local: None
            }]
        );
    }

    #[test]
    fn same_hash_updates_base_only_when_needed() {
        assert_eq!(
            run(&[e("a", Some(1))], &[e("a", Some(1))], &[], false),
            vec![Action::UpdateBase {
                path: "a".into(),
                hash: h(1)
            }]
        );
        assert!(run(&[e("a", Some(1))], &[e("a", Some(1))], &[("a", 1)], false).is_empty());
    }

    #[test]
    fn only_remote_changed_downloads() {
        assert_eq!(
            run(&[e("a", Some(1))], &[e("a", Some(2))], &[("a", 1)], false),
            vec![Action::Download {
                local_path: "a".into(),
                remote_path: "a".into(),
                hash: h(2),
                expected_local: Some(h(1))
            }]
        );
    }

    #[test]
    fn only_local_changed_does_nothing() {
        assert!(run(&[e("a", Some(2))], &[e("a", Some(1))], &[("a", 1)], false).is_empty());
    }

    #[test]
    fn both_changed_loser_creates_conflict_winner_waits() {
        let local = [e("docs/report.docx", Some(2))];
        let remote = [e("docs/report.docx", Some(3))];
        assert_eq!(
            run(&local, &remote, &[("docs/report.docx", 1)], false),
            vec![Action::Conflict {
                local_path: "docs/report.docx".into(),
                conflict_path: "docs/report (conflict-Mac 2026-10-13 1705).docx".into(),
                remote_path: "docs/report.docx".into(),
                hash: h(3),
            }]
        );
        assert!(run(&local, &remote, &[("docs/report.docx", 1)], true).is_empty());
    }

    #[test]
    fn no_base_and_different_is_conflict() {
        let a = run(&[e("a.txt", Some(2))], &[e("a.txt", Some(3))], &[], false);
        assert!(matches!(&a[..], [Action::Conflict { .. }]));
    }

    #[test]
    fn matching_is_case_insensitive_and_keeps_local_name() {
        assert_eq!(
            run(
                &[e("Docs/A.txt", Some(1))],
                &[e("docs/a.txt", Some(2))],
                &[("DOCS/a.TXT", 1)],
                false
            ),
            vec![Action::Download {
                local_path: "Docs/A.txt".into(),
                remote_path: "docs/a.txt".into(),
                hash: h(2),
                expected_local: Some(h(1))
            }]
        );
    }

    #[test]
    fn conflict_names() {
        let s = "2026-10-13 1705";
        assert_eq!(
            conflict_name("a/report.docx", "Mac", s),
            "a/report (conflict-Mac 2026-10-13 1705).docx"
        );
        assert_eq!(
            conflict_name("Makefile", "Mac", s),
            "Makefile (conflict-Mac 2026-10-13 1705)"
        );
        assert_eq!(
            conflict_name(".env", "Mac", s),
            ".env (conflict-Mac 2026-10-13 1705)"
        );
        assert_eq!(
            conflict_name("x.tar.gz", "PC:1/2", s),
            "x.tar (conflict-PC_1_2 2026-10-13 1705).gz"
        );
    }

    #[test]
    fn rename_detection_avoids_re_download() {
        let local = [e("old_name.iso", Some(77))];
        let remote = [e("new_name.iso", Some(77))];
        let actions = run(&local, &remote, &[], false);

        assert_eq!(
            actions,
            vec![Action::Rename {
                from_path: "old_name.iso".into(),
                to_path: "new_name.iso".into(),
                hash: h(77),
            }]
        );
    }

    #[test]
    fn version_vector_resolves_newer_remote_despite_clock_skew() {
        let mut vv_local = VersionVector::new();
        vv_local.set("peer_b", 1);
        let mut local_entry = Entry::simple("shared.doc", 100, 999_999_999_000, Some(h(1)));
        local_entry.vv = vv_local;

        let mut vv_remote = VersionVector::new();
        vv_remote.set("peer_b", 2);
        let mut remote_entry = Entry::simple("shared.doc", 150, 1_000_000, Some(h(2)));
        remote_entry.vv = vv_remote;

        let actions = run(&[local_entry], &[remote_entry], &[], false);
        assert_eq!(
            actions,
            vec![Action::Download {
                local_path: "shared.doc".into(),
                remote_path: "shared.doc".into(),
                hash: h(2),
                expected_local: Some(h(1)),
            }]
        );
    }

    #[test]
    fn version_vector_resolves_newer_local_despite_clock_skew() {
        let mut vv_local = VersionVector::new();
        vv_local.set("dev_a", 3);
        let mut local_entry = Entry::simple("notes.txt", 100, 1_000, Some(h(1)));
        local_entry.vv = vv_local;

        let mut vv_remote = VersionVector::new();
        vv_remote.set("dev_a", 2);
        let mut remote_entry = Entry::simple("notes.txt", 120, 999_999_999_000, Some(h(2)));
        remote_entry.vv = vv_remote;

        let actions = run(&[local_entry], &[remote_entry], &[], false);
        assert!(actions.is_empty(), "Local dominates, no download needed");
    }

    #[test]
    fn version_vector_concurrent_produces_counter_conflict_file() {
        let mut vv_local = VersionVector::new();
        vv_local.set("dev_a", 2);
        let mut local_entry = Entry::simple("code.rs", 50, 100, Some(h(1)));
        local_entry.vv = vv_local;

        let mut vv_remote = VersionVector::new();
        vv_remote.set("dev_b", 1);
        let mut remote_entry = Entry::simple("code.rs", 60, 200, Some(h(2)));
        remote_entry.vv = vv_remote;

        let actions = run(&[local_entry], &[remote_entry], &[], false);
        assert_eq!(
            actions,
            vec![Action::Conflict {
                local_path: "code.rs".into(),
                conflict_path: "code (conflict-Mac 2026-10-13 1705).rs".into(),
                remote_path: "code.rs".into(),
                hash: h(2),
            }]
        );

        let conflict_name = conflict_name_with_counter("code.rs", "MacBook", 3);
        assert_eq!(conflict_name, "code (conflict-MacBook v3).rs");
    }

    #[test]
    fn test_delete_mode_never_ignores_remote_delete() {
        let actions = run_with_mode(
            &[e("doc.pdf", Some(1))],
            &[e("doc.pdf", None)],
            &[("doc.pdf", 1)],
            DeleteMode::Never,
        );
        assert!(
            actions.is_empty(),
            "DeleteMode::Never must ignore remote deletion"
        );
    }

    #[test]
    fn test_delete_mode_mirror_trash_trashes_unmodified_local_file() {
        let actions = run_with_mode(
            &[e("doc.pdf", Some(1))],
            &[e("doc.pdf", None)],
            &[("doc.pdf", 1)],
            DeleteMode::MirrorTrash,
        );
        assert_eq!(
            actions,
            vec![Action::Trash {
                path: "doc.pdf".into()
            }]
        );
    }

    #[test]
    fn test_delete_mode_ask_requests_confirmation() {
        let actions = run_with_mode(
            &[e("doc.pdf", Some(1))],
            &[e("doc.pdf", None)],
            &[("doc.pdf", 1)],
            DeleteMode::Ask,
        );
        assert_eq!(
            actions,
            vec![Action::AskDelete {
                path: "doc.pdf".into()
            }]
        );
    }

    #[test]
    fn test_delete_mode_mirror_trash_with_newer_remote_version_vector() {
        let mut vv_local = VersionVector::new();
        vv_local.set("peer_b", 1);
        let mut local_entry = Entry::simple("photo.jpg", 100, 100, Some(h(1)));
        local_entry.vv = vv_local;

        let mut vv_remote = VersionVector::new();
        vv_remote.set("peer_b", 2);
        let mut remote_entry = Entry::simple("photo.jpg", 0, 0, None);
        remote_entry.vv = vv_remote;

        let actions =
            run_with_mode(&[local_entry], &[remote_entry], &[], DeleteMode::MirrorTrash);
        assert_eq!(
            actions,
            vec![Action::Trash {
                path: "photo.jpg".into()
            }]
        );
    }

    #[test]
    fn test_delete_mode_protects_locally_modified_file() {
        let actions = run_with_mode(
            &[e("doc.pdf", Some(2))],
            &[e("doc.pdf", None)],
            &[("doc.pdf", 1)],
            DeleteMode::MirrorTrash,
        );
        assert!(
            actions.is_empty(),
            "Locally modified file must not be trashed"
        );
    }

    #[test]
    fn test_delete_mode_does_not_trash_file_handled_as_rename() {
        let actions = run_with_mode(
            &[e("old.txt", Some(10))],
            &[e("new.txt", Some(10)), e("old.txt", None)],
            &[],
            DeleteMode::MirrorTrash,
        );
        assert_eq!(
            actions,
            vec![Action::Rename {
                from_path: "old.txt".into(),
                to_path: "new.txt".into(),
                hash: h(10),
            }]
        );
    }
}

mod protocol_tests {
    use lansync_core::protocol::*;
    use tokio::io::AsyncWriteExt;

    fn entry(n: usize) -> Entry {
        Entry::simple(format!("f{n}.txt"), n as u64, 1, Some([n as u8; 32]))
    }

    #[tokio::test]
    async fn frame_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let msgs = vec![
            Message::Hello {
                device_id: "ID".into(),
                name: "Mac".into(),
                proto: PROTO_VERSION,
                pairing: false,
                listen_port: 47800,
            },
            Message::Index {
                entries: vec![entry(1), Entry { hash: None, ..entry(2) }],
            },
            Message::FileChunk {
                data: vec![7; 1000],
            },
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
        assert!(err.to_string().contains("frame too large"), "{err}");
    }

    #[test]
    fn index_messages_batches() {
        let entries: Vec<Entry> = (0..12_001).map(entry).collect();
        let msgs = index_messages(entries, true);
        assert_eq!(msgs.len(), 3);
        assert!(matches!(&msgs[0], Message::Index { entries } if entries.len() == INDEX_BATCH));
        assert!(matches!(&msgs[2], Message::IndexUpdate { entries } if entries.len() == 2_001));
        assert_eq!(
            index_messages(vec![], true),
            vec![Message::Index { entries: vec![] }]
        );
        assert!(index_messages(vec![], false).is_empty());
    }
}

mod transfer_tests {
    use lansync_core::fsutil::{hash_file, mtime_ns};
    use lansync_core::protocol::{read_msg, Message};
    use lansync_core::transfer::*;
    use std::path::Path;

    fn tmp_files(dir: &Path) -> Vec<String> {
        walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".lansync-tmp-") || n.starts_with(".lansync-part-"))
            .collect()
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
        let sender =
            tokio::spawn(async move { send_file(&mut a, &root, "sub/x.bin", &hash).await.unwrap() });

        let Message::FileHeader {
            size,
            hash: h,
            mtime_ns: m,
            ..
        } = read_msg(&mut b).await.unwrap()
        else {
            panic!("expected FileHeader")
        };
        let mut inc = Incoming::begin(dst.path(), "sub/x.bin", size, h, m).await;
        loop {
            match read_msg(&mut b).await.unwrap() {
                Message::FileChunk { data } => inc.chunk(&data).await,
                Message::FileEnd => break,
                other => panic!("unexpected message {other:?}"),
            }
        }
        assert!(sender.await.unwrap());
        let entry = inc.finish(None).await.unwrap();

        assert_eq!(std::fs::read(dst.path().join("sub/x.bin")).unwrap(), data);
        assert_eq!(entry.hash, Some(hash));
        let src_m = mtime_ns(&std::fs::metadata(&sp).unwrap());
        assert!(
            (entry.mtime_ns - src_m).abs() < 1_000,
            "mtime must match sender"
        );
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
        std::fs::write(dst.path().join("a.txt"), b"new local version").unwrap();
        let new = *blake3::hash(b"from remote peer").as_bytes();
        let mut inc = Incoming::begin(dst.path(), "a.txt", 16, new, 0).await;
        inc.chunk(b"from remote peer").await;
        let old = *blake3::hash(b"old version").as_bytes();
        assert!(matches!(
            inc.finish(Some(old)).await,
            Err(FinishError::LocalChanged)
        ));
        assert_eq!(
            std::fs::read(dst.path().join("a.txt")).unwrap(),
            b"new local version"
        );
    }

    #[tokio::test]
    async fn send_file_returns_false_when_missing() {
        let src = tempfile::tempdir().unwrap();
        let (mut a, _b) = tokio::io::duplex(1024);
        assert!(!send_file(&mut a, src.path(), "missing.txt", &[0; 32])
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn incoming_rejects_path_traversal() {
        let dst = tempfile::tempdir().unwrap();
        let mut inc = Incoming::begin(dst.path(), "../../etc/passwd", 5, [0; 32], 0).await;
        inc.chunk(b"hello").await;
        let res = inc.finish(None).await;
        assert!(res.is_err());
        assert!(!dst.path().join("etc/passwd").exists());
        assert!(!dst.path().join("../../etc/passwd").exists());
    }

    #[tokio::test]
    async fn incoming_archives_previous_version_before_overwrite() {
        let dst = tempfile::tempdir().unwrap();
        let initial_data = b"original content to be preserved";
        std::fs::write(dst.path().join("report.txt"), initial_data).unwrap();
        let expected_hash = *blake3::hash(initial_data).as_bytes();

        let new_data = b"incoming overwritten content";
        let new_hash = *blake3::hash(new_data).as_bytes();

        let mut inc =
            Incoming::begin(dst.path(), "report.txt", new_data.len() as u64, new_hash, 1000).await;
        inc.chunk(new_data).await;
        inc.finish(Some(expected_hash)).await.unwrap();

        assert_eq!(
            std::fs::read(dst.path().join("report.txt")).unwrap(),
            new_data
        );

        let versions = lansync_core::versioning::list_versions(dst.path(), "report.txt")
            .await
            .unwrap();
        assert_eq!(versions.len(), 1, "Must have exactly 1 archived backup");
        assert_eq!(std::fs::read(&versions[0].path).unwrap(), initial_data);
    }

    #[tokio::test]
    async fn test_resumable_file_transfer() {
        let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let data: Vec<u8> = (0..500_000u32).map(|i| (i % 251) as u8).collect();
        let hash = *blake3::hash(&data).as_bytes();
        std::fs::write(src.path().join("bigfile.bin"), &data).unwrap();

        {
            let mut inc = Incoming::begin_resumable(
                dst.path(),
                "bigfile.bin",
                data.len() as u64,
                hash,
                1000,
                0,
            )
            .await;
            inc.chunk(&data[..200_000]).await;
            drop(inc);
        }

        let resume_offset =
            check_resume_offset(dst.path(), "bigfile.bin", &hash, data.len() as u64).await;
        assert_eq!(
            resume_offset, 200_000,
            "Must detect 200,000 bytes available for resume"
        );

        let (mut a, mut b) = tokio::io::duplex(256 * 1024);
        let src_path = src.path().to_path_buf();
        let sender = tokio::spawn(async move {
            send_file_range(&mut a, &src_path, "bigfile.bin", &hash, resume_offset)
                .await
                .unwrap();
        });

        let mut inc_resumed = Incoming::begin_resumable(
            dst.path(),
            "bigfile.bin",
            data.len() as u64,
            hash,
            1000,
            resume_offset,
        )
        .await;
        let header = read_msg(&mut b).await.unwrap();
        assert!(matches!(
            header,
            Message::FileHeaderRange {
                offset: 200_000,
                ..
            }
        ));

        loop {
            match read_msg(&mut b).await.unwrap() {
                Message::FileChunk { data } => inc_resumed.chunk(&data).await,
                Message::FileEnd => break,
                other => panic!("Unexpected msg: {:?}", other),
            }
        }
        sender.await.unwrap();

        let entry = inc_resumed.finish(None).await.unwrap();
        assert_eq!(entry.size, 500_000);
        assert_eq!(
            std::fs::read(dst.path().join("bigfile.bin")).unwrap(),
            data
        );
        assert!(
            tmp_files(dst.path()).is_empty(),
            "All temporary part files must be removed"
        );
    }
}

mod transport_tests {
    use lansync_core::identity::Identity;
    use lansync_core::protocol::{read_msg, write_msg, Message};
    use lansync_core::transport::*;
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

mod version_vector_tests {
    use lansync_core::version_vector::*;
    use rand::Rng;

    #[test]
    fn empty_vectors_are_equal() {
        let v1 = VersionVector::new();
        let v2 = VersionVector::new();
        assert_eq!(v1.relation(&v2), VectorRelation::Equal);
    }

    #[test]
    fn single_device_ordering() {
        let mut v1 = VersionVector::new();
        v1.increment("mac");

        let mut v2 = VersionVector::new();
        v2.increment("mac");
        v2.increment("mac");

        assert_eq!(v1.relation(&v2), VectorRelation::Older);
        assert_eq!(v2.relation(&v1), VectorRelation::Newer);
    }

    #[test]
    fn concurrent_modifications() {
        let mut v1 = VersionVector::new();
        v1.increment("mac");

        let mut v2 = VersionVector::new();
        v2.increment("win");

        assert_eq!(v1.relation(&v2), VectorRelation::Concurrent);
        assert_eq!(v2.relation(&v1), VectorRelation::Concurrent);
    }

    #[test]
    fn merge_takes_pairwise_max() {
        let mut v1 = VersionVector::new();
        v1.set("dev_a", 3);
        v1.set("dev_b", 1);

        let mut v2 = VersionVector::new();
        v2.set("dev_a", 2);
        v2.set("dev_b", 5);
        v2.set("dev_c", 4);

        v1.merge(&v2);
        assert_eq!(v1.get("dev_a"), 3);
        assert_eq!(v1.get("dev_b"), 5);
        assert_eq!(v1.get("dev_c"), 4);
        assert_eq!(v1.relation(&v2), VectorRelation::Newer);
    }

    #[test]
    fn randomized_property_test_invariants() {
        let mut rng = rand::thread_rng();
        let devices = ["dev_a", "dev_b", "dev_c", "dev_d"];

        for _ in 0..2_000 {
            let mut v1 = VersionVector::new();
            let mut v2 = VersionVector::new();
            let mut v3 = VersionVector::new();

            for dev in devices {
                if rng.gen_bool(0.7) {
                    v1.set(dev, rng.gen_range(0..10));
                }
                if rng.gen_bool(0.7) {
                    v2.set(dev, rng.gen_range(0..10));
                }
                if rng.gen_bool(0.7) {
                    v3.set(dev, rng.gen_range(0..10));
                }
            }

            assert_eq!(v1.relation(&v1), VectorRelation::Equal);

            let rel_12 = v1.relation(&v2);
            let rel_21 = v2.relation(&v1);
            match rel_12 {
                VectorRelation::Equal => assert_eq!(rel_21, VectorRelation::Equal),
                VectorRelation::Newer => assert_eq!(rel_21, VectorRelation::Older),
                VectorRelation::Older => assert_eq!(rel_21, VectorRelation::Newer),
                VectorRelation::Concurrent => assert_eq!(rel_21, VectorRelation::Concurrent),
            }

            if rel_12 == VectorRelation::Newer && v2.relation(&v3) == VectorRelation::Newer {
                assert_eq!(v1.relation(&v3), VectorRelation::Newer);
            }

            let mut merged = v1.clone();
            merged.merge(&v2);
            assert!(matches!(
                merged.relation(&v1),
                VectorRelation::Equal | VectorRelation::Newer
            ));
            assert!(matches!(
                merged.relation(&v2),
                VectorRelation::Equal | VectorRelation::Newer
            ));
        }
    }
}

mod versioning_tests {
    use lansync_core::versioning::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_archive_and_prune_versions() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("document.txt");

        tokio::fs::write(&file_path, b"version 1 content")
            .await
            .unwrap();
        let b1 = archive_version(dir.path(), "document.txt", 2).await.unwrap();
        assert!(b1.is_some());
        assert!(b1.unwrap().exists());

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        tokio::fs::write(&file_path, b"version 2 content")
            .await
            .unwrap();
        let b2 = archive_version(dir.path(), "document.txt", 2).await.unwrap();
        assert!(b2.is_some());

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        tokio::fs::write(&file_path, b"version 3 content")
            .await
            .unwrap();
        let b3 = archive_version(dir.path(), "document.txt", 2).await.unwrap();
        assert!(b3.is_some());

        let versions = list_versions(dir.path(), "document.txt").await.unwrap();
        assert_eq!(versions.len(), 2, "Must keep at most 2 versions");

        let older_snapshot = &versions[1].path;
        restore_version(dir.path(), "document.txt", older_snapshot)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(&file_path).await.unwrap(),
            b"version 2 content"
        );
    }

    #[tokio::test]
    async fn test_list_all_versioned_files_and_restore_by_timestamp() {
        let dir = tempdir().unwrap();
        let f1 = dir.path().join("notes.txt");
        let f2_dir = dir.path().join("sub");
        tokio::fs::create_dir_all(&f2_dir).await.unwrap();
        let f2 = f2_dir.join("code.rs");

        tokio::fs::write(&f1, b"hello notes").await.unwrap();
        tokio::fs::write(&f2, b"fn main() {}").await.unwrap();

        archive_version(dir.path(), "notes.txt", 5).await.unwrap();
        archive_version(dir.path(), "sub/code.rs", 5).await.unwrap();

        let files = list_all_versioned_files(dir.path());
        assert_eq!(
            files,
            vec!["notes.txt".to_string(), "sub/code.rs".to_string()]
        );

        tokio::fs::write(&f1, b"modified notes").await.unwrap();
        let versions = list_versions(dir.path(), "notes.txt").await.unwrap();
        assert_eq!(versions.len(), 1);

        restore_version_by_timestamp(dir.path(), "notes.txt", versions[0].timestamp_micros)
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(&f1).await.unwrap(), b"hello notes");
    }
}

mod watcher_tests {
    use lansync_core::watcher::FolderWatcher;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn watcher_detects_changes() {
        let dir = tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(10);
        let _watcher = FolderWatcher::start(dir.path(), Duration::from_millis(50), tx).unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::fs::write(dir.path().join("a.txt"), b"123")
            .await
            .unwrap();

        let received = tokio::time::timeout(Duration::from_millis(1500), rx.recv()).await;
        assert!(received.is_ok(), "Watcher must detect newly created file");
    }

    #[tokio::test]
    async fn watcher_skips_ignored_files() {
        let dir = tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(10);
        let _watcher = FolderWatcher::start(dir.path(), Duration::from_millis(50), tx).unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::fs::write(dir.path().join(".DS_Store"), b"junk")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join(".lansync-tmp-abc"), b"junk")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("file.tmp"), b"junk")
            .await
            .unwrap();

        let received = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
        assert!(
            received.is_err(),
            "Watcher must ignore .DS_Store and tmp files"
        );
    }
}
