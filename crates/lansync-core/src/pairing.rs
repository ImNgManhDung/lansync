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

