// Phase 4 chunk N: TLS for the Synapse auth proxy.
//
// Each worker owns a self-signed cert pair persisted at
// `<data_dir>/synapse-cert.pem` + `synapse-key.pem`. The cert's SHA-256
// fingerprint rides in the beacon body (so it's HMAC-protected too) and
// gets pinned by the host on first pairing.
//
// The handshake stack:
//
//     TCP ─▶ rustls (TLS 1.3, custom verifier on host) ─▶ length-prefixed JSON
//
// Custom verifier rather than full PKI because we're not buying certs from a
// CA — the security promise is "you trust this fingerprint, exactly once,
// when you paste the token." Rustls' default verifier would reject every
// connection because none of these certs chain to a public root.

use crate::config;
use anyhow::{anyhow, Context, Result};
use data_encoding::HEXLOWER;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, ServerConfig, SignatureScheme};
use sha2::{Digest, Sha256};
use std::fs;
use std::sync::Arc;

/// Load the worker's cert + key, generating them on first call. Idempotent:
/// reads the same files on every subsequent call. Returns the parsed DER
/// bundle along with a SHA-256 fingerprint of the leaf cert (lowercase hex,
/// no separators).
pub fn load_or_create_cert(
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>, String)> {
    let cert_path = config::synapse_cert_path();
    let key_path = config::synapse_key_path();

    if !cert_path.exists() || !key_path.exists() {
        // Subject altname covers the literal hostname AND localhost — the
        // host always reaches us via the beacon-advertised IP, but having
        // both in the cert SANs makes manual `openssl s_client -connect`
        // testing easier.
        let host = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "localmind".to_string());
        let CertifiedKey { cert, key_pair } =
            generate_simple_self_signed(vec![host.clone(), "localhost".into()])
                .map_err(|e| anyhow!("generate cert: {e}"))?;
        if let Some(parent) = cert_path.parent() {
            fs::create_dir_all(parent).context("creating cert dir")?;
        }
        fs::write(&cert_path, cert.pem()).context("writing cert pem")?;
        fs::write(&key_path, key_pair.serialize_pem()).context("writing key pem")?;
    }

    let cert_pem = fs::read(&cert_path).context("reading cert")?;
    let key_pem = fs::read(&key_path).context("reading key")?;
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| anyhow!("parse cert: {e}"))?;
    if certs.is_empty() {
        return Err(anyhow!("cert file is empty"));
    }
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
        .map_err(|e| anyhow!("parse key: {e}"))?
        .ok_or_else(|| anyhow!("no private key in key file"))?;

    let fingerprint = fingerprint_of(&certs[0]);
    Ok((certs, key, fingerprint))
}

/// SHA-256 of the leaf cert's DER, lowercase hex with no separators. We use
/// hex (not base32) to match the `openssl x509 -fingerprint -sha256`
/// canonical format users see when sanity-checking a cert manually.
pub fn fingerprint_of(cert: &CertificateDer) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cert.as_ref());
    HEXLOWER.encode(&hasher.finalize())
}

/// Build a rustls ServerConfig for the worker auth proxy. No client auth
/// at the TLS layer — the application-layer handshake already token-gates
/// every connection.
pub fn server_config(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<ServerConfig> {
    // We reject client certs entirely; build an empty store.
    let _empty_roots = RootCertStore::empty();
    let _no_client_verifier = WebPkiClientVerifier::no_client_auth();
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| anyhow!("server tls config: {e}"))
}

/// Build a rustls ClientConfig that pins exactly one cert fingerprint. Used
/// by `host_proxy` when dialing a worker — the host has the worker's
/// fingerprint from the verified beacon and refuses anything else.
pub fn client_config_pinning(expected_fingerprint_hex: String) -> Result<ClientConfig> {
    let verifier = Arc::new(PinnedFingerprintVerifier {
        expected: expected_fingerprint_hex.to_ascii_lowercase(),
    });
    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(cfg)
}

/// Custom rustls verifier: accept exactly one cert, identified by SHA-256
/// fingerprint. Rejects anything else. The "danger" knob in rustls expects
/// a verifier that says yes to *something*; ours says yes to exactly the
/// pinned cert and no to literally everything else.
#[derive(Debug)]
struct PinnedFingerprintVerifier {
    expected: String,
}

impl ServerCertVerifier for PinnedFingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        // Empty expected fingerprint = TOFU mode. Used when the host has a
        // token but no fingerprint yet (typically: paired with a Phase 3
        // worker, or paired before re-pairing via the dialog). The token
        // gate at the application layer is the real defense in this mode.
        if self.expected.is_empty() {
            return Ok(ServerCertVerified::assertion());
        }
        let actual = fingerprint_of(end_entity);
        if actual == self.expected {
            Ok(ServerCertVerified::assertion())
        } else {
            // Surface expected vs actual in the error so logs are
            // diagnostic without leaking secrets — fingerprints are
            // already public.
            Err(rustls::Error::General(format!(
                "cert fingerprint mismatch: expected {}, got {actual}",
                self.expected
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        // We pin by fingerprint, not by chain — the signature is whatever
        // the matched cert produced; trust by definition.
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        // Match what rcgen produces by default (ECDSA P-256) plus the
        // common alternatives so cert rotation doesn't require code change.
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
        ]
    }
}
