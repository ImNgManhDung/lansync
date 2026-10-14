//! Self-signed machine certificates and device_id derivation.
use anyhow::Result;
use data_encoding::{BASE32_NOPAD, HEXLOWER};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct Identity {
    pub cert_der: Vec<u8>,
    /// PKCS#8 DER private key
    pub key_der: Vec<u8>,
    pub device_id: String,
}

pub fn device_id_from_cert(cert_der: &[u8]) -> String {
    BASE32_NOPAD.encode(&Sha256::digest(cert_der))[..16].to_string()
}

pub fn fingerprint(cert_der: &[u8]) -> String {
    HEXLOWER.encode(&Sha256::digest(cert_der))
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
        let params = rcgen::CertificateParams::new(vec!["lansync.local".to_string()])?;
        let cert = params.self_signed(&key)?;
        let cert_der = cert.der().to_vec();
        Ok(Self { device_id: device_id_from_cert(&cert_der), cert_der, key_der: key.serialize_der() })
    }

    /// Loads `cert.der` + `key.der` in `dir`, or generates a new pair if missing.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let (cp, kp) = (dir.join("cert.der"), dir.join("key.der"));
        if cp.exists() && kp.exists() {
            let cert_der = std::fs::read(&cp)?;
            return Ok(Self { device_id: device_id_from_cert(&cert_der), cert_der, key_der: std::fs::read(&kp)? });
        }
        std::fs::create_dir_all(dir)?;
        let id = Self::generate()?;
        std::fs::write(&cp, &id.cert_der)?;
        std::fs::write(&kp, &id.key_der)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&kp, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_16_base32_chars() {
        let id = Identity::generate().unwrap();
        assert_eq!(id.device_id.len(), 16);
        assert!(id.device_id.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
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
        assert_eq!(fingerprint(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
