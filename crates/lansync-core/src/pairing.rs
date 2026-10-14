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

    pub fn add(&mut self, peer: Peer) {
        self.peers.retain(|p| p.device_id != peer.device_id);
        self.peers.push(peer);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.len();
        self.peers.retain(|p| p.device_id != id);
        before != self.len()
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
        Peer { device_id: id.into(), name: format!("Device {id}"), cert_fingerprint: fp.into(), last_addr: None }
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
}
