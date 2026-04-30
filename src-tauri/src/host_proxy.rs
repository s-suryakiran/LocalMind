// Phase 3 chunk C: host-side local proxy.
//
// llama-server doesn't speak our auth handshake — it just dials TCP and
// starts the rpc protocol. So when the host wants to use authenticated
// workers, we interpose. For each (endpoint, token) pair the user has
// configured, we bind a local listener on 127.0.0.1:<random>. llama-server
// gets pointed at that local address. When it connects, this proxy:
//   1. dials the remote worker
//   2. completes the synapse handshake using the worker's token
//   3. bidirectionally forwards bytes
//
// Architecture (one connection's lifecycle):
//
//                                                 ┌────────────────────────────┐
//                                                 │ remote worker auth_proxy   │
//                                                 │     (0.0.0.0:50052)        │
//                                                 └─────────────▲──────────────┘
//                                                               │ handshake
//                                  ┌─────────────┐              │ + raw bytes
//   llama-server ──TCP──▶ 127.0.0.1:NNNN ──┐    │ host_proxy ──┘
//                                          └──▶ │   (this file)
//                                               └─────────────┘
//
// One local listener per (endpoint, token) pair. They live for the lifetime
// of the loaded model: created in `LlamaState::start`, torn down in `stop`.
// We pick the local port via 127.0.0.1:0 so multiple workers can coexist
// without a port allocator.

use crate::synapse_proto::{self, HandshakeRequest, PROTOCOL_VERSION};
use crate::synapse_tls;
use anyhow::{anyhow, bail, Result};
use rustls::pki_types::ServerName;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

pub struct HostProxy {
    /// Active local listeners, one per worker. We just need to abort them on
    /// shutdown — the listener tasks own their own state. A Vec is fine; the
    /// pool churns at most once per model load and stays small (few workers).
    proxies: Mutex<Vec<JoinHandle<()>>>,
}

impl HostProxy {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            proxies: Mutex::new(Vec::new()),
        })
    }

    /// Stand up a local listener that authenticates and forwards to `remote`.
    /// Returns the local address llama-server should connect to instead of
    /// the remote endpoint (e.g. `"127.0.0.1:54712"`).
    ///
    /// Performs a probe handshake first so authentication failures surface
    /// at start-of-load rather than on first inference token. If the probe
    /// fails, no listener is bound and the error bubbles up — caller should
    /// abort the whole start_llama call so the user sees the failure
    /// directly rather than ending up with a half-loaded model that can't
    /// actually serve.
    pub async fn start(
        &self,
        app: &AppHandle,
        remote: &str,
        token: &str,
        fingerprint: Option<&str>,
    ) -> Result<String> {
        if token.trim().is_empty() {
            bail!(
                "no token configured for worker {remote} — paste the worker's token in the Synapse tab"
            );
        }

        // Phase 4 chunk N: a missing fingerprint isn't fatal — older workers
        // (Phase 3) didn't advertise one, and the user might not have
        // re-paired since updating. Without the pin, we still negotiate
        // TLS but accept any cert (TOFU). The token still gates access
        // application-side, so this isn't a regression vs Phase 3.
        let fingerprint_owned = fingerprint.map(|s| s.to_ascii_lowercase());

        // Probe handshake: dial, handshake, hang up. Cheap (~1 RTT) and gives
        // immediate, correct error messages for the three failure modes a
        // user actually hits: wrong token, unreachable worker, version mismatch.
        probe_handshake(remote, token, fingerprint_owned.as_deref())
            .await
            .map_err(|e| anyhow!("auth probe to {remote}: {e}"))?;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| anyhow!("local proxy bind: {e}"))?;
        let local_port = listener
            .local_addr()
            .map_err(|e| anyhow!("local addr: {e}"))?
            .port();
        let local = format!("127.0.0.1:{local_port}");

        log_info(
            app,
            &format!("local proxy {local} → {remote} (auth OK, probe verified)"),
        );

        // Phase 4 chunk K: shared health flag flipped by the pinger and
        // read by the listener. When the worker is evicted (heartbeat
        // failed for 30s+), accepting a connection and trying to forward
        // would hang llama-server's dial — better to reject fast with a
        // clean TCP RST so the user gets an immediate "connection refused"
        // and can debug. Recovery flips it back automatically.
        let healthy = Arc::new(AtomicBool::new(true));
        let healthy_listener = healthy.clone();
        let healthy_pinger = healthy.clone();

        let remote_owned = remote.to_string();
        let token_owned = token.to_string();
        let fingerprint_listener = fingerprint_owned.clone();
        let app_outer = app.clone();
        let task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((client, _peer)) => {
                        if !healthy_listener.load(Ordering::Relaxed) {
                            // Drop immediately. We don't even handshake —
                            // this saves llama-server from waiting on a
                            // probe to a dead worker. Closing the socket
                            // gives it a clean RST inside ~1 ms.
                            drop(client);
                            log_warn(
                                &app_outer,
                                &format!(
                                    "local proxy → {remote_owned}: refusing client (worker offline)"
                                ),
                            );
                            continue;
                        }
                        let remote = remote_owned.clone();
                        let token = token_owned.clone();
                        let fingerprint = fingerprint_listener.clone();
                        let app = app_outer.clone();
                        tokio::spawn(async move {
                            if let Err(e) =
                                forward(client, &remote, &token, fingerprint.as_deref()).await
                            {
                                // One bad connection shouldn't tear the proxy
                                // down. Surface the error via the live-logs
                                // stream so users can see if e.g. their worker
                                // restarted and rotated the token.
                                log_warn(&app, &format!("local proxy → {remote}: {e}"));
                            }
                        });
                    }
                    Err(e) => {
                        log_warn(&app_outer, &format!("local proxy accept: {e}"));
                        // Avoid busy-looping on a permanently-broken listener.
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
            }
        });

        self.proxies.lock().await.push(task);

        // Phase 3 chunk F2: spawn a per-worker RTT pinger. We re-do the
        // handshake every 5s and emit the round-trip in ms as a
        // `synapse:rtt` event. Cheap (~1 RTT, no actual rpc traffic) and
        // gives the UI a live signal of network health independent of
        // whether inference is running. Bundled into the same JoinHandle
        // pool so stop_all() takes both down together.
        //
        // Phase 4 chunk I: count consecutive failures. After EVICT_AFTER
        // misses (~30s of silence) we emit `synapse:proxy-evicted` and stop
        // pinging. The proxy listener stays up so a future model load still
        // surfaces a clear "auth probe to X" error rather than silently
        // missing a worker. Chunk K (reconnect) will re-arm the pinger when
        // the worker comes back.
        let remote_ping = remote.to_string();
        let token_ping = token.to_string();
        let fingerprint_ping = fingerprint_owned.clone();
        let app_ping = app.clone();
        let pinger = tokio::spawn(async move {
            const EVICT_AFTER: u32 = 6;
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
            // First tick fires immediately; skip it so we don't pile a probe
            // on top of the start-of-load handshake the caller already did.
            ticker.tick().await;
            let mut consecutive_fails: u32 = 0;
            let mut evicted = false;
            loop {
                ticker.tick().await;
                let started = std::time::Instant::now();
                let result =
                    probe_handshake(&remote_ping, &token_ping, fingerprint_ping.as_deref()).await;
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let ok = result.is_ok();
                let _ = app_ping.emit(
                    "synapse:rtt",
                    serde_json::json!({
                        "endpoint": &remote_ping,
                        "rttMs": elapsed_ms,
                        "ok": ok,
                        "ts": now_ms(),
                    }),
                );
                if ok {
                    if evicted {
                        // Worker came back — flip the listener back to
                        // accepting connections, emit recovery so UI
                        // restores its place in the active list, and reset
                        // the ticker to its normal cadence.
                        healthy_pinger.store(true, Ordering::Relaxed);
                        let _ = app_ping.emit(
                            "synapse:proxy-recovered",
                            serde_json::json!({ "endpoint": &remote_ping, "ts": now_ms() }),
                        );
                        evicted = false;
                        ticker = tokio::time::interval(std::time::Duration::from_secs(5));
                        ticker.tick().await; // burn immediate tick
                    }
                    consecutive_fails = 0;
                } else {
                    consecutive_fails = consecutive_fails.saturating_add(1);
                    if consecutive_fails == EVICT_AFTER && !evicted {
                        evicted = true;
                        // Flip listener BEFORE emitting so a racing client
                        // connection sees the unhealthy state.
                        healthy_pinger.store(false, Ordering::Relaxed);
                        let _ = app_ping.emit(
                            "synapse:proxy-evicted",
                            serde_json::json!({
                                "endpoint": &remote_ping,
                                "afterMs": (EVICT_AFTER as u64) * 5_000,
                                "ts": now_ms(),
                            }),
                        );
                        // Slow the ticker after eviction so we don't hammer
                        // a sleeping worker — once a minute is plenty for
                        // detecting "it's back."
                        ticker = tokio::time::interval(std::time::Duration::from_secs(60));
                        ticker.tick().await; // burn the immediate tick
                    }
                }
            }
        });
        self.proxies.lock().await.push(pinger);

        Ok(local)
    }

    /// Tear down every active local proxy. Called from `LlamaState::stop`
    /// so that unloading a model frees both VRAM AND the local proxy ports.
    pub async fn stop_all(&self) {
        let mut p = self.proxies.lock().await;
        for handle in p.drain(..) {
            handle.abort();
        }
    }
}

/// Open a TLS-wrapped TCP connection to the remote auth proxy. The TLS
/// verifier pins the cert fingerprint when one is supplied; without one
/// we accept any cert (TOFU) and rely on the application-layer token to
/// gate access. Both probe and forward go through here for consistency.
async fn open_tls(remote: &str, fingerprint: Option<&str>) -> Result<TlsStream<TcpStream>> {
    let (host_str, _port_str) = remote
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("malformed endpoint {remote} (need host:port)"))?;
    // rustls needs a SNI name even though our pinned verifier ignores
    // the chain. Pass the literal hostname/IP the user typed.
    let server_name = ServerName::try_from(host_str.to_string())
        .map_err(|e| anyhow!("invalid TLS server name {host_str}: {e}"))?;

    // Connect timeout matters — a stale IP shouldn't make the user wait
    // for the OS default (~75s). 5s is plenty for a healthy LAN.
    let tcp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        TcpStream::connect(remote),
    )
    .await
    .map_err(|_| anyhow!("connect timeout (5s) — is the worker reachable?"))?
    .map_err(|e| anyhow!("connect: {e}"))?;

    let cfg = match fingerprint {
        Some(fp) => synapse_tls::client_config_pinning(fp.to_string())?,
        // Empty pin = TOFU. The verifier rejects every non-empty cert
        // against an empty fingerprint, so we'd never connect; instead
        // pass a sentinel that matches anything by short-circuiting.
        None => synapse_tls::client_config_pinning(String::new())?,
    };
    let connector = TlsConnector::from(Arc::new(cfg));
    let tls = connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| anyhow!("tls handshake: {e}"))?;
    Ok(tls)
}

async fn probe_handshake(remote: &str, token: &str, fingerprint: Option<&str>) -> Result<()> {
    let mut sock = open_tls(remote, fingerprint).await?;
    let req = HandshakeRequest {
        v: PROTOCOL_VERSION,
        token: token.to_string(),
        client_ts_ms: Some(now_ms()),
    };
    synapse_proto::write_handshake_request(&mut sock, &req).await?;
    let res = synapse_proto::read_handshake_response(&mut sock).await?;
    if !res.ok {
        bail!(res
            .reason
            .unwrap_or_else(|| "rejected (no reason given)".into()));
    }
    Ok(())
}

async fn forward(
    client: TcpStream,
    remote: &str,
    token: &str,
    fingerprint: Option<&str>,
) -> Result<()> {
    let mut upstream = open_tls(remote, fingerprint).await?;
    let req = HandshakeRequest {
        v: PROTOCOL_VERSION,
        token: token.to_string(),
        client_ts_ms: Some(now_ms()),
    };
    synapse_proto::write_handshake_request(&mut upstream, &req).await?;
    let res = synapse_proto::read_handshake_response(&mut upstream).await?;
    if !res.ok {
        bail!(
            "auth rejected by {remote}: {}",
            res.reason.unwrap_or_else(|| "(no reason)".into())
        );
    }
    let mut client = client;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
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
