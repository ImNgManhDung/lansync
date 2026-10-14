//! 6-digit pairing code and paired peer store (peers.json).
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Symmetric 6-digit code derived identically on both peers if both observe the correct certificates.
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
    #[serde(default)]
    pub delete_mode: Option<crate::config::DeleteMode>,
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
        // Atomic write via temp file + rename to prevent corruption on unexpected shutdown
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.peers)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn list(&self) -> &[Peer] {
        &self.peers
    }

    pub fn get(&self, id: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.device_id == id)
    }

    pub fn add(&mut self, peer: Peer) {
        self.peers.retain(|p| p.device_id != peer.device_id);
        self.peers.push(peer);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.len();
        self.peers.retain(|p| p.device_id != id);
        before != self.len()
    }

    pub fn set_delete_mode(&mut self, id: &str, mode: Option<crate::config::DeleteMode>) -> bool {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.device_id == id) {
            peer.delete_mode = mode;
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairingLimiterError {
    #[error("Pairing locked out for {remaining_secs}s due to too many failed attempts")]
    LockedOut { remaining_secs: u64 },
    #[error("Maximum pairing attempts exceeded")]
    MaxAttemptsExceeded,
    #[error("Pairing code expired")]
    CodeExpired,
}

#[derive(Debug, Clone)]
struct AttemptRecord {
    failed_attempts: u32,
    lockout_until: Option<std::time::Instant>,
    last_attempt: std::time::Instant,
}

pub struct PairingLimiter {
    max_attempts: u32,
    code_ttl: std::time::Duration,
    lockout_duration: std::time::Duration,
    attempts: std::collections::HashMap<String, AttemptRecord>,
}

impl PairingLimiter {
    pub fn new(max_attempts: u32, code_ttl: std::time::Duration, lockout_duration: std::time::Duration) -> Self {
        Self {
            max_attempts,
            code_ttl,
            lockout_duration,
            attempts: std::collections::HashMap::new(),
        }
    }

    pub fn default_policy() -> Self {
        Self::new(10, std::time::Duration::from_secs(90), std::time::Duration::from_secs(30))
    }

    pub fn reset(&mut self, peer_key: &str) {
        self.attempts.remove(peer_key);
    }

    pub fn code_ttl(&self) -> std::time::Duration {
        self.code_ttl
    }

    pub fn can_attempt(&self, peer_key: &str, now: std::time::Instant) -> Result<(), PairingLimiterError> {
        if let Some(record) = self.attempts.get(peer_key) {
            if let Some(lockout) = record.lockout_until {
                if now < lockout {
                    let remaining_secs = (lockout - now).as_secs().max(1);
                    return Err(PairingLimiterError::LockedOut { remaining_secs });
                }
                // Lockout has ended, peer can try again
                return Ok(());
            }
            if record.failed_attempts >= self.max_attempts {
                return Err(PairingLimiterError::MaxAttemptsExceeded);
            }
        }
        Ok(())
    }

    pub fn is_expired(&self, created_at: std::time::Instant, now: std::time::Instant) -> bool {
        now.saturating_duration_since(created_at) > self.code_ttl
    }

    pub fn record_failure(&mut self, peer_key: &str, now: std::time::Instant) {
        let record = self.attempts.entry(peer_key.to_string()).or_insert_with(|| AttemptRecord {
            failed_attempts: 0,
            lockout_until: None,
            last_attempt: now,
        });

        if record.lockout_until.is_some_and(|lockout| now >= lockout) {
            record.failed_attempts = 0;
            record.lockout_until = None;
        }

        record.failed_attempts += 1;
        record.last_attempt = now;

        if record.failed_attempts >= self.max_attempts {
            let backoff_multiplier = record.failed_attempts - self.max_attempts + 1;
            let duration = self.lockout_duration * backoff_multiplier;
            record.lockout_until = Some(now + duration);
        }
    }

    pub fn record_success(&mut self, peer_key: &str) {
        self.attempts.remove(peer_key);
    }

    pub fn cleanup(&mut self, now: std::time::Instant) {
        let max_age = std::time::Duration::from_secs(3600);
        self.attempts.retain(|_, r| {
            if let Some(lockout) = r.lockout_until {
                now < lockout
            } else {
                now.saturating_duration_since(r.last_attempt) < max_age
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        s.add(peer("A", "fp3"));           // Re-pair -> replace old entry
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

    #[test]
    fn test_peer_delete_mode_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let mut s = PeerStore::load(&path).unwrap();
        s.add(peer("dev1", "fp1"));
        assert_eq!(s.list()[0].delete_mode, None);

        assert!(s.set_delete_mode("dev1", Some(crate::config::DeleteMode::MirrorTrash)));
        assert_eq!(s.list()[0].delete_mode, Some(crate::config::DeleteMode::MirrorTrash));
        s.save().unwrap();

        let loaded = PeerStore::load(&path).unwrap();
        assert_eq!(loaded.list()[0].delete_mode, Some(crate::config::DeleteMode::MirrorTrash));
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
