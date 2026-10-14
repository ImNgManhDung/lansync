//! TCP + Mutual TLS 1.3 authentication using self-signed certificates.
use crate::identity::Identity;
use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

pub struct SecureConn {
    pub stream: TlsStream<TcpStream>,
    /// Remote peer DER certificate
    pub peer_cert: Vec<u8>,
    pub peer_addr: SocketAddr,
}

/// Accepts any certificate while validating handshake signatures.
/// The engine validates the fingerprint against the paired peers list.
#[derive(Debug)]
struct AcceptAnyCert {
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime)
        -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

impl ClientCertVerifier for AcceptAnyCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime)
        -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

fn own_cert(id: &Identity) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    (
        vec![CertificateDer::from(id.cert_der.clone())],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key_der.clone())),
    )
}

pub fn server_config(id: &Identity) -> Result<Arc<ServerConfig>> {
    let p = provider();
    let verifier = Arc::new(AcceptAnyCert { algs: p.signature_verification_algorithms });
    let (certs, key) = own_cert(id);
    let cfg = ServerConfig::builder_with_provider(p)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)?;
    Ok(Arc::new(cfg))
}

pub fn client_config(id: &Identity) -> Result<Arc<ClientConfig>> {
    let p = provider();
    let verifier = Arc::new(AcceptAnyCert { algs: p.signature_verification_algorithms });
    let (certs, key) = own_cert(id);
    let cfg = ClientConfig::builder_with_provider(p)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(certs, key)?;
    Ok(Arc::new(cfg))
}

pub async fn connect(addr: SocketAddr, cfg: Arc<ClientConfig>) -> Result<SecureConn> {
    let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr))
        .await
        .context("connection timed out")??;
    tcp.set_nodelay(true)?;
    let name = ServerName::try_from("lansync.local")?.to_owned();
    let tls = TlsConnector::from(cfg).connect(name, tcp).await?;
    let peer_cert = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).map(|c| c.to_vec())
        .context("remote peer did not provide a certificate")?;
    Ok(SecureConn { stream: tls.into(), peer_cert, peer_addr: addr })
}

pub async fn accept(tcp: TcpStream, cfg: Arc<ServerConfig>) -> Result<SecureConn> {
    tcp.set_nodelay(true)?;
    let peer_addr = tcp.peer_addr()?;
    let tls = TlsAcceptor::from(cfg).accept(tcp).await?;
    let peer_cert = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).map(|c| c.to_vec())
        .context("remote peer did not provide a certificate")?;
    Ok(SecureConn { stream: tls.into(), peer_cert, peer_addr })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::protocol::{read_msg, write_msg, Message};
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
