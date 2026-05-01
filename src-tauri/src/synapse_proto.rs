// Phase 3 wire protocol for the worker auth proxy and host-side local proxy.
// Some helpers (HandshakeRequest writer, HMAC verify) are still consumer-
// less until chunks C/E land — keep the module-scope dead_code allow for
// just those couple items rather than per-fn churn. Each fn is kept pub so
// downstream chunks don't have to re-export.
#![allow(dead_code)]

// Synapse Phase 3 wire protocol for the worker auth proxy.
//
// On the worker, an `auth_proxy` task listens on the public 0.0.0.0:50052 and
// proxies authenticated bytes to a localhost-only rpc-server. On the host,
// per-peer local proxies do the matching handshake before forwarding bytes
// from llama-server.
//
// The handshake is a single round-trip:
//
//   client → server: 4-byte BE len + JSON HandshakeRequest
//   server → client: 4-byte BE len + JSON HandshakeResponse
//
// If `ok == true`, both sides start raw bidirectional copy. If `ok == false`,
// the server closes the connection. Wire format is JSON+length so we can
// extend it (TLS handshake hooks, capability flags, version bumps) without
// breaking older peers; protocol version `1` is the only currently valid one.
//
// Beacon signing (separate from this handshake): the UDP/mDNS announce
// payload includes an `hmac` field — HMAC-SHA256 of the canonical body using
// the same token. Hosts who hold the token can verify they're talking to a
// legitimate worker; hosts who don't see the peer as "unverified" in the UI.

use anyhow::{anyhow, bail, Result};
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const PROTOCOL_VERSION: u8 = 1;
/// Cap on the JSON envelope so a malicious peer can't make us allocate
/// gigabytes by claiming a huge frame.
const MAX_FRAME_BYTES: usize = 64 * 1024;

/// 256-bit token; 32 bytes is the comfortable point where birthday collisions
/// stop being a thing and base32 still encodes to a humane 52 chars.
const TOKEN_BYTES: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeRequest {
    pub v: u8,
    /// Base32-encoded token presented by the client. Compared in constant
    /// time on the server so token brute-force can't time-leak.
    pub token: String,
    /// Optional client-supplied unix-millis so the server can echo it back
    /// in a future ping extension. Currently unused; reserved.
    #[serde(default)]
    pub client_ts_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeResponse {
    pub v: u8,
    pub ok: bool,
    /// Human-readable failure reason. Always set when `ok == false`; safe to
    /// log but should NOT be shown verbatim to end users — it intentionally
    /// echoes details (e.g. "version mismatch") that aren't sensitive.
    #[serde(default)]
    pub reason: Option<String>,
    /// Server unix-millis, for round-trip-time computation on the client.
    #[serde(default)]
    pub server_ts_ms: Option<u64>,
}

/// Generate a new 256-bit token, base32-encoded (no padding). Caller is
/// responsible for persistence.
pub fn generate_token() -> String {
    let mut buf = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut buf);
    BASE32_NOPAD.encode(&buf)
}

/// Constant-time compare of two tokens. Both sides are normalized to upper
/// case (base32 alphabet is case-insensitive) and trimmed of whitespace
/// before comparison so copy-paste artifacts don't cause spurious failures.
pub fn tokens_eq(a: &str, b: &str) -> bool {
    let a = a.trim().to_ascii_uppercase();
    let b = b.trim().to_ascii_uppercase();
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        bail!("frame too large: {len} bytes (max {MAX_FRAME_BYTES})");
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

pub async fn write_frame<W>(writer: &mut W, payload: &[u8]) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    if payload.len() > MAX_FRAME_BYTES {
        bail!("frame too large: {} bytes", payload.len());
    }
    let len = (payload.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_handshake_request<R>(reader: &mut R) -> Result<HandshakeRequest>
where
    R: AsyncReadExt + Unpin,
{
    let bytes = read_frame(reader).await?;
    let req: HandshakeRequest =
        serde_json::from_slice(&bytes).map_err(|e| anyhow!("bad handshake JSON: {e}"))?;
    if req.v != PROTOCOL_VERSION {
        bail!(
            "unsupported protocol version {} (need {PROTOCOL_VERSION})",
            req.v
        );
    }
    Ok(req)
}

pub async fn write_handshake_response<W>(writer: &mut W, res: &HandshakeResponse) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let bytes = serde_json::to_vec(res)?;
    write_frame(writer, &bytes).await
}

pub async fn read_handshake_response<R>(reader: &mut R) -> Result<HandshakeResponse>
where
    R: AsyncReadExt + Unpin,
{
    let bytes = read_frame(reader).await?;
    let res: HandshakeResponse =
        serde_json::from_slice(&bytes).map_err(|e| anyhow!("bad handshake response: {e}"))?;
    Ok(res)
}

pub async fn write_handshake_request<W>(writer: &mut W, req: &HandshakeRequest) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let bytes = serde_json::to_vec(req)?;
    write_frame(writer, &bytes).await
}

/// HMAC-SHA256 of `payload` keyed by the base32 token. Used by both sides
/// for beacon signature verification — workers sign their announce body,
/// hosts who hold the token verify before trusting the entry.
pub fn hmac_sign(token: &str, payload: &[u8]) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(token.trim().to_ascii_uppercase().as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(payload);
    BASE32_NOPAD.encode(&mac.finalize().into_bytes())
}

/// Constant-time HMAC verification.
pub fn hmac_verify(token: &str, payload: &[u8], signature: &str) -> bool {
    let expected = hmac_sign(token, payload);
    if expected.len() != signature.len() {
        return false;
    }
    expected.as_bytes().ct_eq(signature.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrip_eq() {
        let t = generate_token();
        assert!(tokens_eq(&t, &t));
        assert!(tokens_eq(&t, &t.to_ascii_lowercase()));
        assert!(tokens_eq(&format!("  {t}  "), &t));
    }

    #[test]
    fn token_eq_rejects_different_lengths() {
        assert!(!tokens_eq("AAAA", "AAAAA"));
        assert!(!tokens_eq("", "X"));
    }

    #[test]
    fn hmac_signs_and_verifies() {
        let token = generate_token();
        let body = b"hello world";
        let sig = hmac_sign(&token, body);
        assert!(hmac_verify(&token, body, &sig));
        assert!(!hmac_verify(&token, b"goodbye world", &sig));
        assert!(!hmac_verify("wrong-token", body, &sig));
    }

    #[tokio::test]
    async fn frame_roundtrip() {
        let payload = b"sup".to_vec();
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &payload).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let got = read_frame(&mut cursor).await.unwrap();
        assert_eq!(got, payload);
    }
}
