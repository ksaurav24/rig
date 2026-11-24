//! TLS fingerprint probing and Trust-On-First-Use (TOFU) certificate pinning.
//!
//! Enables Rig Desktop (and CLI) to connect safely to `lan` / `--tls internal`
//! servers with self-signed Caddy internal CA certificates by pinning the leaf
//! certificate's SHA-256 fingerprint across REST, SSE, and WebSocket streams.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::error::{ClientError, ClientResult};

/// Result of probing a server's TLS endpoint before pairing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsProbe {
    pub is_https: bool,
    /// Colon-separated uppercase SHA-256 hex fingerprint of the leaf cert.
    pub fingerprint_sha256: Option<String>,
    /// True if the certificate verifies against standard webpki root CAs.
    pub trusted_by_roots: bool,
}

/// Normalizes a SHA-256 fingerprint to lowercase hex without colons/spaces.
pub fn normalize_fingerprint(fp: &str) -> String {
    fp.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Formats raw SHA-256 bytes as `AA:BB:CC:...` for human inspection.
pub fn format_fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn provider() -> Arc<CryptoProvider> {
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()))
}

#[derive(Debug)]
struct PinnedCertVerifier {
    expected_hex: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let actual = hex::encode(Sha256::digest(end_entity.as_ref()));
        if actual == self.expected_hex {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::General(format!(
                "pinned TLS fingerprint mismatch: expected {}, got {}",
                self.expected_hex, actual
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[derive(Debug)]
struct ProbeCertVerifier {
    captured: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for ProbeCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let fp = format_fingerprint(end_entity.as_ref());
        if let Ok(mut guard) = self.captured.lock() {
            *guard = Some(fp);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Builds a `rustls::ClientConfig` that pins a specific SHA-256 leaf cert fingerprint.
pub fn pinned_tls_config(fingerprint_sha256: &str) -> ClientResult<ClientConfig> {
    let expected_hex = normalize_fingerprint(fingerprint_sha256);
    if expected_hex.len() != 64 {
        return Err(ClientError::message(
            "TLS certificate fingerprint must be a 64-hex-character SHA-256 digest",
        ));
    }
    let prov = provider();
    let verifier = Arc::new(PinnedCertVerifier {
        expected_hex,
        provider: prov.clone(),
    });
    let cfg = ClientConfig::builder_with_provider(prov)
        .with_safe_default_protocol_versions()
        .map_err(|e| ClientError::message(format!("initializing TLS config: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(cfg)
}

/// Builds a standard webpki-roots `rustls::ClientConfig`.
pub fn standard_tls_config() -> ClientResult<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let prov = provider();
    let cfg = ClientConfig::builder_with_provider(prov)
        .with_safe_default_protocol_versions()
        .map_err(|e| ClientError::message(format!("initializing TLS config: {e}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(cfg)
}

/// Probes a Rig server URL to check whether it uses HTTPS, retrieve its leaf
/// certificate SHA-256 fingerprint, and determine whether standard root CAs
/// trust it (or if it is a self-signed / internal CA certificate).
pub async fn probe_tls(base_url: &str) -> ClientResult<TlsProbe> {
    let parsed = url::Url::parse(base_url)
        .map_err(|e| ClientError::message(format!("invalid server URL: {e}")))?;
    if parsed.scheme() == "http" {
        return Ok(TlsProbe {
            is_https: false,
            fingerprint_sha256: None,
            trusted_by_roots: false,
        });
    }
    if parsed.scheme() != "https" {
        return Err(ClientError::message(format!(
            "unsupported URL scheme '{}'",
            parsed.scheme()
        )));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ClientError::message("URL is missing a hostname"))?
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);
    let server_name = ServerName::try_from(host.clone())
        .map_err(|_| ClientError::message(format!("invalid TLS host '{host}'")))?;

    // 1. Check whether standard roots trust the server.
    let std_cfg = Arc::new(standard_tls_config()?);
    let tcp1 = tokio::time::timeout(Duration::from_secs(6), TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| ClientError::network(format!("timed out connecting to {host}:{port}")))?
        .map_err(|e| ClientError::network(format!("could not connect to {host}:{port}: {e}")))?;
    let std_res = tokio::time::timeout(
        Duration::from_secs(6),
        TlsConnector::from(std_cfg).connect(server_name.clone(), tcp1),
    )
    .await;
    if let Ok(Ok(stream)) = std_res {
        let fp = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .map(|cert| format_fingerprint(cert.as_ref()));
        return Ok(TlsProbe {
            is_https: true,
            fingerprint_sha256: fp,
            trusted_by_roots: true,
        });
    }

    // 2. Otherwise inspect the leaf certificate for TOFU fingerprint display.
    let captured = Arc::new(Mutex::new(None));
    let prov = provider();
    let probe_cfg = ClientConfig::builder_with_provider(prov.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| ClientError::message(format!("initializing probe TLS config: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ProbeCertVerifier {
            captured: captured.clone(),
            provider: prov,
        }))
        .with_no_client_auth();
    let tcp2 = tokio::time::timeout(Duration::from_secs(6), TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| ClientError::network(format!("timed out connecting to {host}:{port}")))?
        .map_err(|e| ClientError::network(format!("could not connect to {host}:{port}: {e}")))?;
    let _ = tokio::time::timeout(
        Duration::from_secs(6),
        TlsConnector::from(Arc::new(probe_cfg)).connect(server_name, tcp2),
    )
    .await
    .map_err(|_| ClientError::network(format!("TLS handshake timed out for {host}:{port}")))?
    .map_err(|e| ClientError::network(format!("TLS handshake failed for {host}:{port}: {e}")))?;

    let fp = captured.lock().ok().and_then(|g| g.clone());
    Ok(TlsProbe {
        is_https: true,
        fingerprint_sha256: fp,
        trusted_by_roots: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_normalization_and_formatting() {
        let fp = format_fingerprint(b"hello-cert");
        assert_eq!(fp.split(':').count(), 32);
        let norm = normalize_fingerprint(&fp);
        assert_eq!(norm.len(), 64);
        assert!(norm.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }
}
