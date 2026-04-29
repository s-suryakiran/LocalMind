// Phase 3 chunk B: worker-side auth proxy.
//
// rpc-server itself has no authentication and isn't safe to expose on a
// network with untrusted clients (llama.cpp's own startup banner says so).
// Instead, the worker now binds rpc-server to 127.0.0.1:<internal> and stands
// this proxy on 0.0.0.0:<public>. Clients must complete the synapse_proto
// handshake with the worker's token before any byte reaches rpc-server.
//
// Architecture:
//
//   external client ──TCP─▶ 0.0.0.0:50052 (auth_proxy)
//                                  │
//                          handshake (token check)
//                                  │
//                                  ├── ok  ──▶ tokio::io::copy_bidirectional
//                                  │           ──▶ 127.0.0.1:51052 (rpc-server)
//                                  │
//                                  └── bad ──▶ HandshakeResponse { ok: false }
//                                              + close
//
// Each accepted client gets its own task. A failed handshake never opens a
// connection to rpc-server, so a brute-force attacker can't even probe the
// upstream protocol.

use crate::synapse_proto::{self, HandshakeResponse, PROTOCOL_VERSION};
use anyhow::{anyhow, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// What we tell the UI when a client tries to connect. Front-end mirrors
/// these into the live-logs panel and a future "active connections" badge.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthEvent {
    pub kind: AuthEventKind,
    pub peer: String,
    /// Optional human-readable detail — populated on rejections so the
    /// worker UI can show *why* a host couldn't connect.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthEventKind {
    Accepted,
    Rejected,
    Disconnected,
    UpstreamFailed,
}

/// Spawn the auth proxy as a tokio task. Returns the JoinHandle so the
/// caller can `.abort()` on stop_worker. The listener binds synchronously
/// so any port-already-in-use error surfaces before this returns.
pub async fn spawn_auth_proxy(
    app: AppHandle,
    public_port: u16,
    internal_port: u16,
    token: String,
) -> Result<JoinHandle<()>> {
    let listener = TcpListener::bind(("0.0.0.0", public_port))
        .await
        .map_err(|e| anyhow!("auth proxy bind 0.0.0.0:{public_port}: {e}"))?;
    log_info(
        &app,
        &format!("auth proxy listening on 0.0.0.0:{public_port} → 127.0.0.1:{internal_port}"),
    );

    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((client, peer)) => {
                    let app = app.clone();
                    let token = token.clone();
                    tokio::spawn(async move {
                        let peer_str = peer.to_string();
                        if let Err(e) =
                            handle_client(client, &token, internal_port, &app, &peer_str).await
                        {
                            // Swallow the error here so a single misbehaving
                            // peer doesn't take down the whole proxy. Most
                            // hits are normal — port scanners, half-open
                            // connections, etc.
                            log_warn(&app, &format!("auth: peer {peer_str} dropped: {e}"));
                        }
                    });
                }
                Err(e) => {
                    log_warn(&app, &format!("auth proxy accept failed: {e}"));
                    // Don't busy-loop if the listener is permanently broken.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    });
    Ok(handle)
}

async fn handle_client(
    mut client: TcpStream,
    expected_token: &str,
    internal_port: u16,
    app: &AppHandle,
    peer: &str,
) -> Result<()> {
    // Phase 1: handshake. We expect the client to send a length-prefixed
    // HandshakeRequest within a short timeout — anything slower is almost
    // certainly a port scanner or a confused client speaking the wrong
    // protocol, and we don't want such peers to keep the listener busy.
    let req = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        synapse_proto::read_handshake_request(&mut client),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            emit_auth(
                app,
                AuthEventKind::Rejected,
                peer,
                Some(format!("malformed handshake: {e}")),
            );
            return Ok(());
        }
        Err(_) => {
            emit_auth(
                app,
                AuthEventKind::Rejected,
                peer,
                Some("handshake timeout (5s)".into()),
            );
            return Ok(());
        }
    };

    let ok = synapse_proto::tokens_eq(&req.token, expected_token);
    let response = HandshakeResponse {
        v: PROTOCOL_VERSION,
        ok,
        reason: if ok {
            None
        } else {
            Some("invalid token".into())
        },
        server_ts_ms: Some(now_ms()),
    };
    if let Err(e) = synapse_proto::write_handshake_response(&mut client, &response).await {
        emit_auth(
            app,
            AuthEventKind::Rejected,
            peer,
            Some(format!("response write failed: {e}")),
        );
        return Ok(());
    }
    if !ok {
        emit_auth(
            app,
            AuthEventKind::Rejected,
            peer,
            Some("invalid token".into()),
        );
        // Half-flush so the client actually sees the rejection before TCP
        // RST eats it. shutdown() also signals EOF so the client's read
        // loop exits cleanly.
        let _ = client.shutdown().await;
        return Ok(());
    }

    // Phase 2: dial rpc-server on localhost. If this fails the worker process
    // is broken (rpc-server crashed?) — we tell the client and bail.
    let upstream = match TcpStream::connect(("127.0.0.1", internal_port)).await {
        Ok(s) => s,
        Err(e) => {
            emit_auth(
                app,
                AuthEventKind::UpstreamFailed,
                peer,
                Some(format!("dial rpc-server: {e}")),
            );
            return Err(anyhow!("upstream dial failed: {e}"));
        }
    };

    emit_auth(app, AuthEventKind::Accepted, peer, None);

    // Phase 3: bidirectional byte copy. tokio's helper is the right tool —
    // it shuts the opposite half down when one side EOFs, which matches
    // how llama.cpp's RPC stream terminates. We propagate no errors here;
    // both directions can independently end and that's normal.
    let mut client = client;
    let mut upstream = upstream;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;

    emit_auth(app, AuthEventKind::Disconnected, peer, None);
    Ok(())
}

fn emit_auth(app: &AppHandle, kind: AuthEventKind, peer: &str, reason: Option<String>) {
    let evt = AuthEvent {
        kind,
        peer: peer.to_string(),
        reason: reason.clone(),
    };
    let _ = app.emit("synapse:auth", &evt);
    // Mirror to the live-logs stream so users see auth activity inline with
    // rpc-server output instead of needing a separate UI surface.
    let line = match reason {
        Some(r) => format!("auth {kind:?}: {peer} ({r})"),
        None => format!("auth {kind:?}: {peer}"),
    };
    let stream = match kind {
        AuthEventKind::Accepted | AuthEventKind::Disconnected => "stdout",
        AuthEventKind::Rejected | AuthEventKind::UpstreamFailed => "stderr",
    };
    let _ = app.emit(
        "synapse:log",
        serde_json::json!({ "stream": stream, "line": line }),
    );
}

fn log_info(app: &AppHandle, line: &str) {
    let _ = app.emit(
        "synapse:log",
        serde_json::json!({ "stream": "stdout", "line": line }),
    );
}

fn log_warn(app: &AppHandle, line: &str) {
    let _ = app.emit(
        "synapse:log",
        serde_json::json!({ "stream": "stderr", "line": line }),
    );
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
