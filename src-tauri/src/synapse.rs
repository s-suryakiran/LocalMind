// Phase 2 Synapse: each machine can run an `rpc-server` child process that
// llama.cpp on a remote host reaches via `--rpc host:port`. On top of Phase 1's
// manual start/stop, we now also:
//   - advertise the worker on the LAN via mDNS (`_localmind-synapse._tcp`)
//   - browse for other workers, emitting peer add/remove events to the UI
//   - expose a `restart_worker` so the host can flush worker VRAM on demand
//
// Auth + smart layer split + tok/s telemetry come in Phase 3.
use crate::{auth_proxy, binaries, config, synapse_token};
use anyhow::{anyhow, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UdpSocket;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub const DEFAULT_WORKER_PORT: u16 = 50052;
const SERVICE_TYPE: &str = "_localmind-synapse._tcp.local.";

/// Internal port that rpc-server binds to on localhost. Public traffic hits
/// the auth proxy on `public_port`, which forwards here after the handshake
/// succeeds. We pick `public_port + 1000` so it's predictable for log
/// reading + clearly outside the user-facing port range.
fn internal_rpc_port(public_port: u16) -> u16 {
    public_port.saturating_add(1000)
}

// UDP broadcast beacon — runs alongside mDNS as a fallback for networks
// where multicast is blocked (very common on Wi-Fi). Worker sends every
// BEACON_INTERVAL; host listens on BEACON_PORT and ages peers out after
// PEER_TTL of silence.
const BEACON_PORT: u16 = 50053;
const BEACON_INTERVAL: Duration = Duration::from_secs(3);
const PEER_TTL: Duration = Duration::from_secs(15);
const BEACON_MAGIC: &str = "localmind-synapse/1";

/// What we serialize and HMAC over. The signature itself rides outside this
/// type (in `SignedBeacon`) so verification can canonicalize the body bytes
/// independently of the JSON field order. Keeping the body stable means we
/// can add fields later without breaking signatures across versions.
///
/// Phase 4 chunk J: hardware fields (vram_gb, accelerator_kind) so the host
/// can show real per-worker capacity in the marketplace and Synapse UI
/// instead of the 4-GB-per-peer guess from Phase 3. All hardware fields are
/// optional so older workers (no fields → None) keep working with no
/// special-casing on the host side.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BeaconPayload {
    /// Magic string so we can ignore stray UDP traffic on this port.
    magic: String,
    /// Stable per-machine ID so the host can dedupe.
    id: String,
    hostname: String,
    port: u16,
    version: String,
    /// VRAM (or unified memory share) advertised in GB, rounded to 1 decimal.
    /// Hosts use this to size their Synapse-cluster memory budget.
    #[serde(default)]
    vram_gb: Option<f32>,
    /// Short tag for the accelerator kind: "apple", "nvidia", "amd",
    /// "intel-arc", "cpu". UI uses it to pick an icon.
    #[serde(default)]
    accelerator_kind: Option<String>,
    /// Friendly accelerator description ("Apple M1 Pro", "NVIDIA RTX 5070 Ti")
    /// for the chip subtitle.
    #[serde(default)]
    accelerator_name: Option<String>,
    /// Phase 4 chunk N: SHA-256 hex fingerprint of the worker's TLS cert.
    /// Rides inside the HMAC-signed body so a man-in-the-middle can't
    /// substitute their own fingerprint without invalidating the signature.
    #[serde(default)]
    cert_fingerprint: Option<String>,
}

/// On-the-wire envelope. `body` carries the user-visible fields; `hmac` is
/// HMAC-SHA256(token, raw-body-bytes). Hosts that hold the token verify;
/// hosts that don't see the peer as `verified: false`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedBeacon {
    body: BeaconPayload,
    /// Base32-encoded HMAC of the raw `body` JSON bytes. Empty/missing on
    /// pre-Phase-3 workers — verification just fails closed in that case.
    #[serde(default)]
    hmac: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SynapseWorkerStatus {
    pub running: bool,
    pub port: u16,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SynapsePeer {
    /// Stable mDNS instance name — used as the dedupe key.
    pub id: String,
    /// Human-friendly hostname (whatever the worker advertised).
    pub hostname: String,
    /// First resolvable address — usually the LAN IPv4 we want.
    pub address: String,
    pub port: u16,
    /// `address:port`, the exact string the host appends to `--rpc`.
    pub endpoint: String,
    /// Phase 3 chunk E: true iff the host already has a token for this
    /// endpoint AND the beacon's HMAC verifies under it. Hosts with no token
    /// see this as false — they can still add the peer manually, the dialog
    /// captures the token, and once stored verification flips on.
    pub verified: bool,
    /// Phase 4 chunk J: hardware advertised in the beacon. Pre-Phase-4
    /// workers leave these as None.
    pub vram_gb: Option<f32>,
    pub accelerator_kind: Option<String>,
    pub accelerator_name: Option<String>,
    /// Phase 4 chunk N: SHA-256 fingerprint of the worker's TLS cert.
    /// The host pins this on first pair so a same-IP attacker can't
    /// impersonate the worker even with the token.
    pub cert_fingerprint: Option<String>,
}

struct WorkerHandle {
    child: Option<Child>,
    port: u16,
    /// Owns the mDNS advertisement; dropping unregisters automatically.
    advertised: Option<ServiceInfo>,
    /// Beacon broadcaster task; cancelled when the worker stops.
    beacon: Option<JoinHandle<()>>,
    /// Auth proxy task that fronts rpc-server on the public port. Cancelled
    /// in stop_worker so the public port frees up immediately.
    auth_proxy: Option<JoinHandle<()>>,
}

/// Beacon entry tracked on the host side. We only emit a peer-added event
/// when we first hear from a worker, then refresh `last_seen` on each ping.
/// A janitor task removes entries that have gone silent for PEER_TTL.
///
/// `last_body` and `last_hmac` are kept so `set_known_tokens` can re-verify
/// the cached signature when a host adds a token after the fact — without
/// them, the user would have to wait up to BEACON_INTERVAL for the next
/// signed beacon to flip the badge from unverified to verified.
struct BeaconEntry {
    peer: SynapsePeer,
    last_seen: Instant,
    last_body: Option<Vec<u8>>,
    last_hmac: String,
}

pub struct SynapseState {
    worker: Mutex<WorkerHandle>,
    /// Single shared mDNS daemon (advertise + browse share one socket).
    daemon: Mutex<Option<ServiceDaemon>>,
    /// Peers we've seen, keyed by mDNS instance name. Cached so the UI can
    /// re-render on demand without waiting for the next browse tick.
    peers: Mutex<HashMap<String, SynapsePeer>>,
    /// UDP-beacon peers, keyed by beacon `id`. Separate map so the janitor
    /// can age them out without touching mDNS-discovered ones (mDNS does
    /// its own TTL via ServiceRemoved events).
    beacons: Mutex<HashMap<String, BeaconEntry>>,
    /// Phase 3 chunk E: tokens the host knows about, keyed by endpoint
    /// (`host:port`). Populated from the frontend's persisted store via
    /// `set_known_tokens` when the Synapse page mounts. Used to verify
    /// incoming beacon HMACs — peers with a matching token verify true.
    peer_tokens: Mutex<HashMap<String, String>>,
}

impl SynapseState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            worker: Mutex::new(WorkerHandle {
                child: None,
                port: DEFAULT_WORKER_PORT,
                advertised: None,
                beacon: None,
                auth_proxy: None,
            }),
            daemon: Mutex::new(None),
            peers: Mutex::new(HashMap::new()),
            beacons: Mutex::new(HashMap::new()),
            peer_tokens: Mutex::new(HashMap::new()),
        })
    }

    /// Replace the host-side token map. Called from the frontend whenever
    /// the user adds/edits a worker. Re-verifies cached peers in place so
    /// the UI's verified badges flip immediately, without waiting for the
    /// next beacon tick.
    pub async fn set_known_tokens(&self, tokens: HashMap<String, String>) {
        *self.peer_tokens.lock().await = tokens;
        // Re-verify everything we already know about. Beacon entries store
        // their last-seen body bytes so we can re-check HMAC without waiting
        // 3 seconds for the next broadcast.
        let token_map = self.peer_tokens.lock().await.clone();
        let mut beacons = self.beacons.lock().await;
        for entry in beacons.values_mut() {
            entry.peer.verified = entry
                .last_body
                .as_ref()
                .and_then(|body| token_map.get(&entry.peer.endpoint).map(|t| (body, t)))
                .map(|(body, token)| {
                    crate::synapse_proto::hmac_verify(token, body, &entry.last_hmac)
                })
                .unwrap_or(false);
        }
        // mDNS entries don't carry signatures yet (mdns-sd TXT records have
        // size limits we don't want to fight in this pass), so they always
        // remain unverified. They show alongside beacon entries in the UI;
        // the user can still add them, just without the verified badge.
    }

    pub async fn status(&self) -> SynapseWorkerStatus {
        let w = self.worker.lock().await;
        SynapseWorkerStatus {
            running: w.child.is_some(),
            port: w.port,
            pid: w.child.as_ref().and_then(|c| c.id()),
        }
    }

    pub async fn list_peers(&self) -> Vec<SynapsePeer> {
        // Merge mDNS-discovered peers with UDP-beacon peers, keyed by endpoint
        // so the same worker reachable via both routes shows up only once.
        let mut by_endpoint: HashMap<String, SynapsePeer> = HashMap::new();
        for p in self.peers.lock().await.values() {
            by_endpoint.insert(p.endpoint.clone(), p.clone());
        }
        for entry in self.beacons.lock().await.values() {
            by_endpoint
                .entry(entry.peer.endpoint.clone())
                .or_insert_with(|| entry.peer.clone());
        }
        by_endpoint.into_values().collect()
    }

    pub async fn stop_worker(&self) -> Result<()> {
        let mut w = self.worker.lock().await;
        if let Some(mut c) = w.child.take() {
            let _ = c.kill().await;
            let _ = c.wait().await;
        }
        if let Some(handle) = w.beacon.take() {
            handle.abort();
        }
        if let Some(handle) = w.auth_proxy.take() {
            handle.abort();
        }
        if let Some(info) = w.advertised.take() {
            if let Some(d) = self.daemon.lock().await.as_ref() {
                let _ = d.unregister(info.get_fullname());
            }
        }
        Ok(())
    }

    pub async fn start_worker(
        &self,
        app: &AppHandle,
        port: Option<u16>,
    ) -> Result<SynapseWorkerStatus> {
        // Make sure the llama.cpp bundle is unpacked — `rpc-server` ships in
        // the same archive as `llama-server`, so we trigger that download here
        // if the user toggles worker mode before they've ever loaded a model.
        binaries::ensure_llama_server(app).await?;

        self.stop_worker().await?;
        let public_port = port.unwrap_or(DEFAULT_WORKER_PORT);
        let internal_port = internal_rpc_port(public_port);
        // Both ports might be left over from a previous crash. Clean public
        // for the auth proxy and internal for rpc-server itself.
        crate::llama::kill_orphan_on_port(public_port).await;
        crate::llama::kill_orphan_on_port(internal_port).await;

        let binary = config::rpc_server_path();
        if !binary.exists() {
            return Err(anyhow!(
                "rpc-server binary not found at {} — is the bundled llama.cpp build missing RPC support?",
                binary.display()
            ));
        }

        // Phase 3: bind rpc-server to 127.0.0.1 only. The world reaches us
        // through the auth proxy (below), which gates by token before any
        // byte hits this process. llama.cpp's startup banner about "never
        // expose the RPC server" is now satisfied — we never do.
        let mut cmd = Command::new(&binary);
        cmd.arg("-H").arg("127.0.0.1");
        cmd.arg("-p").arg(internal_port.to_string());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn rpc-server: {e}"))?;
        pipe_output(app, &mut child);

        // Spin up the auth proxy on the public-facing port. If this fails
        // (port in use, firewall, etc.) we kill the rpc-server child we just
        // spawned — leaving it running on localhost would create a confusing
        // half-up state where the worker burns RAM but nobody can reach it.
        let token = synapse_token::load_or_create()
            .map_err(|e| anyhow!("failed to load worker token: {e}"))?;
        let proxy_handle = match auth_proxy::spawn_auth_proxy(
            app.clone(),
            public_port,
            internal_port,
            token,
        )
        .await
        {
            Ok(h) => h,
            Err(e) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(e);
            }
        };

        // Advertise on mDNS so the host's Synapse page picks us up automatically.
        // Best-effort: if advertising fails (e.g. no multicast on this NIC, or
        // hostname has chars mdns-sd refuses) the worker still works, the host
        // just has to type the IP manually. Surface the failure as a synapse:log
        // line so the UI can show *why* discovery isn't happening.
        let advertised = match self.advertise(public_port).await {
            Ok(info) => {
                let addrs = info
                    .get_addresses()
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let _ = app.emit(
                    "synapse:log",
                    serde_json::json!({
                        "stream": "stdout",
                        "line": format!(
                            "mDNS advertise OK: {} on {} (port {})",
                            info.get_fullname(),
                            if addrs.is_empty() { "<no addr>".to_string() } else { addrs },
                            public_port,
                        ),
                    }),
                );
                Some(info)
            }
            Err(e) => {
                let msg = format!("mDNS advertise failed: {e}");
                eprintln!("synapse: {msg}");
                let _ = app.emit(
                    "synapse:log",
                    serde_json::json!({ "stream": "stderr", "line": msg }),
                );
                None
            }
        };

        // Spawn the UDP-broadcast beacon. This runs in parallel with mDNS as a
        // fallback for networks where multicast is filtered (Wi-Fi APs with
        // client isolation, Windows machines on Public profile, corp Wi-Fi…).
        // Beacon uses 255.255.255.255, which is far more permissive on most
        // networks than 224.0.0.251 multicast.
        let beacon_handle = match spawn_beacon(app.clone(), public_port).await {
            Ok(h) => Some(h),
            Err(e) => {
                let msg = format!("UDP beacon failed: {e}");
                eprintln!("synapse: {msg}");
                let _ = app.emit(
                    "synapse:log",
                    serde_json::json!({ "stream": "stderr", "line": msg }),
                );
                None
            }
        };

        {
            let mut w = self.worker.lock().await;
            w.child = Some(child);
            w.port = public_port;
            w.advertised = advertised;
            w.beacon = beacon_handle;
            w.auth_proxy = Some(proxy_handle);
        }

        let _ = app.emit("synapse:ready", serde_json::json!({ "port": public_port }));

        Ok(self.status().await)
    }

    /// Stop + start in one call so the worker frees VRAM and re-advertises.
    /// Useful when a previous host disconnected mid-inference and llama.cpp
    /// left buffers allocated.
    pub async fn restart_worker(
        &self,
        app: &AppHandle,
        port: Option<u16>,
    ) -> Result<SynapseWorkerStatus> {
        // If no port given, reuse the one we were last running on.
        let port = match port {
            Some(p) => Some(p),
            None => Some(self.worker.lock().await.port),
        };
        self.start_worker(app, port).await
    }

    /// Start browsing for `_localmind-synapse._tcp` peers. Idempotent — calling
    /// twice keeps the same daemon. Emits `synapse:peer-added` /
    /// `synapse:peer-removed` events as the LAN view changes.
    pub async fn start_discovery(self: &Arc<Self>, app: &AppHandle) -> Result<()> {
        let daemon = {
            let mut guard = self.daemon.lock().await;
            if guard.is_none() {
                *guard = Some(ServiceDaemon::new().map_err(|e| anyhow!("mdns daemon: {e}"))?);
            }
            guard.as_ref().unwrap().clone()
        };

        let receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| anyhow!("mdns browse: {e}"))?;
        // Clone for the mDNS browse task; the original `app` is reused below
        // for the beacon listener + janitor.
        let app_mdns = app.clone();
        let me = self.clone();
        tokio::spawn(async move {
            let app = app_mdns;
            // mdns-sd's receiver is a flume channel; recv_async is awaitable.
            while let Ok(event) = receiver.recv_async().await {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let id = info.get_fullname().to_string();

                        // Skip our own advertisement — nothing useful about
                        // adding ourselves to our own peer list.
                        let own_fullname = me
                            .worker
                            .lock()
                            .await
                            .advertised
                            .as_ref()
                            .map(|a| a.get_fullname().to_string());
                        if own_fullname.as_deref() == Some(id.as_str()) {
                            continue;
                        }

                        let port = info.get_port();
                        let address = info
                            .get_addresses()
                            .iter()
                            .filter(|a| a.is_ipv4())
                            .map(|a| a.to_string())
                            .next()
                            .unwrap_or_else(|| {
                                info.get_addresses()
                                    .iter()
                                    .map(|a| a.to_string())
                                    .next()
                                    .unwrap_or_default()
                            });
                        if address.is_empty() {
                            continue;
                        }
                        let hostname = info
                            .get_property_val_str("hostname")
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| info.get_hostname().to_string());
                        let endpoint = format!("{}:{}", address, port);
                        let peer = SynapsePeer {
                            id: id.clone(),
                            hostname,
                            address,
                            port,
                            endpoint,
                            // mDNS TXT records don't carry our HMAC (size
                            // limits + cache-friendliness). Beacons are the
                            // authenticated channel; mDNS-only peers stay
                            // unverified in the UI.
                            verified: false,
                            // mDNS path also doesn't carry the new chunk-J
                            // hardware fields — those ride only on the
                            // signed beacon. The same peer typically
                            // surfaces via both routes; list_peers dedupes
                            // by endpoint and keeps the beacon's richer
                            // metadata.
                            vram_gb: None,
                            accelerator_kind: None,
                            accelerator_name: None,
                            cert_fingerprint: None,
                        };
                        me.peers.lock().await.insert(id, peer.clone());
                        let _ = app.emit("synapse:peer-added", &peer);
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        let removed = me.peers.lock().await.remove(&fullname).is_some();
                        if removed {
                            let _ = app.emit(
                                "synapse:peer-removed",
                                serde_json::json!({ "id": fullname }),
                            );
                        }
                    }
                    _ => {}
                }
            }
        });

        // UDP-beacon listener. Runs alongside mDNS so peers from either route
        // both surface in the UI (deduped by endpoint in `list_peers`). Bound
        // to 0.0.0.0:50053 — no special firewall coordination needed if the
        // user already let LocalMind through, since this is the same exe.
        match UdpSocket::bind(("0.0.0.0", BEACON_PORT)).await {
            Ok(sock) => {
                let _ = app.emit(
                    "synapse:log",
                    serde_json::json!({
                        "stream": "stdout",
                        "line": format!("UDP beacon listener on 0.0.0.0:{BEACON_PORT}"),
                    }),
                );
                let app_l = app.clone();
                let me_l = self.clone();
                tokio::spawn(async move { run_beacon_listener(me_l, app_l, sock).await });

                // Janitor: drop beacon entries after PEER_TTL of silence so a
                // worker that vanishes (machine slept, network dropped, app
                // killed) doesn't linger forever in the host's peer list.
                let app_j = app.clone();
                let me_j = self.clone();
                tokio::spawn(async move { run_beacon_janitor(me_j, app_j).await });
            }
            Err(e) => {
                let msg = format!("UDP beacon listener failed: {e}");
                eprintln!("synapse: {msg}");
                let _ = app.emit(
                    "synapse:log",
                    serde_json::json!({ "stream": "stderr", "line": msg }),
                );
            }
        }

        Ok(())
    }

    async fn advertise(&self, port: u16) -> Result<ServiceInfo> {
        // Spin up the daemon if start_discovery hasn't been called yet.
        let daemon = {
            let mut guard = self.daemon.lock().await;
            if guard.is_none() {
                *guard = Some(ServiceDaemon::new().map_err(|e| anyhow!("mdns daemon: {e}"))?);
            }
            guard.as_ref().unwrap().clone()
        };

        // Real hostname for the TXT record (UI display) — may contain spaces,
        // underscores, etc. Windows in particular often has hostnames mdns-sd's
        // strict validator rejects.
        let raw_host = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "localmind".to_string());

        // Sanitized hostname for the actual mDNS record: ASCII alnum + hyphen
        // only, falls back to `localmind` if everything was stripped. RFC 1123
        // labels also can't start/end with a hyphen, so we trim those too.
        let sanitized = sanitize_dns_label(&raw_host);
        let dns_host = if sanitized.is_empty() {
            "localmind".to_string()
        } else {
            sanitized
        };
        let instance = format!("{}-{}", dns_host, port);
        let mdns_host = format!("{}.local.", dns_host);

        let ip = local_ip_address::local_ip()
            .map(|ip| ip.to_string())
            .map_err(|e| anyhow!("local ip: {e}"))?;

        let mut props: HashMap<String, String> = HashMap::new();
        props.insert("hostname".to_string(), raw_host.clone());
        props.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());
        props.insert("kind".to_string(), "rpc-server".to_string());

        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &mdns_host,
            ip.as_str(),
            port,
            Some(props),
        )
        .map_err(|e| anyhow!("mdns service info: {e}"))?;

        daemon
            .register(info.clone())
            .map_err(|e| anyhow!("mdns register: {e}"))?;
        Ok(info)
    }
}

/// Coerce an arbitrary hostname into a valid DNS label per RFC 1123:
/// ASCII letters, digits, and hyphens, no leading/trailing hyphen. Windows
/// hostnames may contain underscores or spaces which mdns-sd rejects.
fn sanitize_dns_label(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    cleaned.trim_matches('-').to_string()
}

/// Spawn the UDP-broadcast beacon for this worker. Sends a small JSON packet
/// to 255.255.255.255:BEACON_PORT every BEACON_INTERVAL. Returns a JoinHandle
/// the caller stores so it can `.abort()` on stop_worker.
async fn spawn_beacon(app: AppHandle, port: u16) -> Result<JoinHandle<()>> {
    let raw_host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localmind".to_string());
    let id = format!("beacon:{}-{}", sanitize_dns_label(&raw_host), port);
    // Phase 4 chunk J: snapshot hardware once at start. We don't refresh
    // mid-session — VRAM doesn't change, and re-detecting on every tick
    // would be wasted CPU. If a user moves models around the host's
    // marketplace re-renders on the next set_known_tokens push anyway.
    let hw = crate::hardware::detect();
    let (acc_kind, acc_name, vram_gb) = match &hw.accelerator {
        crate::hardware::Accelerator::AppleSilicon {
            chip,
            unified_memory_gb,
        } => (
            Some("apple".to_string()),
            Some(format!("Apple {chip}")),
            // Apple unified memory: budget the GPU usefully gets is
            // ~75% before macOS pushes back hard. Match what the host
            // marketplace heuristic uses for self.
            Some((*unified_memory_gb as f32) * 0.75),
        ),
        crate::hardware::Accelerator::Nvidia { name, vram_gb, .. } => (
            Some("nvidia".to_string()),
            Some(name.clone()),
            Some(*vram_gb as f32),
        ),
        crate::hardware::Accelerator::Amd { name, vram_gb } => (
            Some("amd".to_string()),
            Some(name.clone()),
            Some(*vram_gb as f32),
        ),
        crate::hardware::Accelerator::IntelArc { name } => {
            (Some("intel-arc".to_string()), Some(name.clone()), None)
        }
        crate::hardware::Accelerator::Cpu => (
            Some("cpu".to_string()),
            Some(hw.cpu_name.clone()),
            // CPU-only: advertise system RAM minus a 4 GB OS reserve so
            // the host's budget calc treats this peer like a CPU node.
            Some(((hw.total_memory_gb - 4.0).max(0.0)) as f32),
        ),
    };
    // Phase 4 chunk N: include the cert fingerprint so paired hosts can
    // pin TLS verification on next connect. Best-effort — if cert load
    // fails (read-only data dir, e.g.), advertise None and the host falls
    // back to TOFU.
    let cert_fingerprint = match crate::synapse_tls::load_or_create_cert() {
        Ok((_, _, fp)) => Some(fp),
        Err(e) => {
            eprintln!("synapse: TLS cert load failed: {e}");
            None
        }
    };
    let payload = BeaconPayload {
        magic: BEACON_MAGIC.to_string(),
        id,
        hostname: raw_host,
        port,
        version: env!("CARGO_PKG_VERSION").to_string(),
        vram_gb,
        accelerator_kind: acc_kind,
        accelerator_name: acc_name,
        cert_fingerprint,
    };
    // Phase 3 chunk E: HMAC-sign the canonical body. Hosts that have our
    // token verify; hosts that don't see us as unverified. The signature
    // is computed over the body bytes specifically (not the wrapped
    // SignedBeacon) so JSON field re-ordering can't affect it.
    let body_bytes = serde_json::to_vec(&payload).map_err(|e| anyhow!("beacon body: {e}"))?;
    let token = crate::synapse_token::load_or_create().map_err(|e| anyhow!("beacon token: {e}"))?;
    let hmac = crate::synapse_proto::hmac_sign(&token, &body_bytes);
    let signed = SignedBeacon {
        body: payload,
        hmac,
    };
    let bytes = serde_json::to_vec(&signed).map_err(|e| anyhow!("beacon serialize: {e}"))?;

    // Bind to an ephemeral port; we only ever send. Setting broadcast on the
    // socket lets us hit the limited-broadcast address 255.255.255.255.
    let sock = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .map_err(|e| anyhow!("beacon bind: {e}"))?;
    sock.set_broadcast(true)
        .map_err(|e| anyhow!("beacon set_broadcast: {e}"))?;

    let _ = app.emit(
        "synapse:log",
        serde_json::json!({
            "stream": "stdout",
            "line": format!(
                "UDP beacon broadcasting on 255.255.255.255:{BEACON_PORT} every {}s",
                BEACON_INTERVAL.as_secs(),
            ),
        }),
    );

    let handle = tokio::spawn(async move {
        loop {
            // Send to the limited broadcast address; routers won't forward it
            // off the LAN, which is exactly what we want.
            if let Err(e) = sock.send_to(&bytes, ("255.255.255.255", BEACON_PORT)).await {
                let _ = app.emit(
                    "synapse:log",
                    serde_json::json!({
                        "stream": "stderr",
                        "line": format!("beacon send failed: {e}"),
                    }),
                );
            }
            tokio::time::sleep(BEACON_INTERVAL).await;
        }
    });
    Ok(handle)
}

/// Receive UDP beacons forever. Each packet that parses cleanly and isn't from
/// our own beacon ID becomes a peer-added (or refresh) event.
async fn run_beacon_listener(state: Arc<SynapseState>, app: AppHandle, sock: UdpSocket) {
    let own_id = format!(
        "beacon:{}-{}",
        sanitize_dns_label(
            &hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "localmind".to_string())
        ),
        state.worker.lock().await.port
    );

    let mut buf = [0u8; 1024];
    loop {
        let (n, src) = match sock.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Try the new SignedBeacon shape first; if that fails, parse as a
        // bare BeaconPayload for backwards compatibility with workers still
        // running pre-chunk-E builds. Either way the peer shows up — the
        // unsigned variant just stays unverified.
        let (payload, body_bytes, hmac) = match parse_beacon(&buf[..n]) {
            Some(parsed) => parsed,
            None => continue,
        };
        if payload.magic != BEACON_MAGIC {
            continue;
        }
        if payload.id == own_id {
            continue;
        }

        let address = src.ip().to_string();
        let endpoint = format!("{}:{}", address, payload.port);

        // Verify the signature if we already have the worker's token. Hosts
        // that haven't paired yet see verified=false; once the user pastes
        // the token via the dialog, set_known_tokens flips this to true on
        // the cached entries without waiting for the next beacon.
        let verified = if hmac.is_empty() {
            false
        } else {
            let tokens = state.peer_tokens.lock().await;
            tokens
                .get(&endpoint)
                .map(|t| crate::synapse_proto::hmac_verify(t, &body_bytes, &hmac))
                .unwrap_or(false)
        };

        let peer = SynapsePeer {
            id: payload.id.clone(),
            hostname: payload.hostname.clone(),
            address,
            port: payload.port,
            endpoint,
            verified,
            vram_gb: payload.vram_gb,
            accelerator_kind: payload.accelerator_kind.clone(),
            accelerator_name: payload.accelerator_name.clone(),
            cert_fingerprint: payload.cert_fingerprint.clone(),
        };

        // Insert or refresh. Only fire peer-added on first sight; refreshes
        // are silent so the UI doesn't churn every 3s.
        let mut beacons = state.beacons.lock().await;
        let is_new = !beacons.contains_key(&payload.id);
        beacons.insert(
            payload.id.clone(),
            BeaconEntry {
                peer: peer.clone(),
                last_seen: Instant::now(),
                last_body: Some(body_bytes),
                last_hmac: hmac,
            },
        );
        drop(beacons);
        if is_new {
            let _ = app.emit("synapse:peer-added", &peer);
        }
    }
}

/// Parse a beacon datagram. Tries the signed envelope first, then falls back
/// to a bare body for older workers. Returns `(payload, body-bytes, hmac)` —
/// the body bytes are needed to re-verify HMAC later when a token arrives.
fn parse_beacon(buf: &[u8]) -> Option<(BeaconPayload, Vec<u8>, String)> {
    if let Ok(signed) = serde_json::from_slice::<SignedBeacon>(buf) {
        // Re-serialize the body bytes in canonical form so verification on a
        // later set_known_tokens() call uses the exact same input the worker
        // signed. We can't trust the original packet bytes here because
        // the SignedBeacon envelope wrapped them with the hmac field.
        if let Ok(body_bytes) = serde_json::to_vec(&signed.body) {
            return Some((signed.body, body_bytes, signed.hmac));
        }
    }
    if let Ok(payload) = serde_json::from_slice::<BeaconPayload>(buf) {
        if let Ok(body_bytes) = serde_json::to_vec(&payload) {
            return Some((payload, body_bytes, String::new()));
        }
    }
    None
}

/// Periodically prune beacon entries whose last_seen is older than PEER_TTL.
/// Removed peers fire `synapse:peer-removed` so the UI can drop them from the
/// list without waiting for a full refresh.
async fn run_beacon_janitor(state: Arc<SynapseState>, app: AppHandle) {
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;
        let mut to_remove = Vec::new();
        {
            let now = Instant::now();
            let mut beacons = state.beacons.lock().await;
            beacons.retain(|id, entry| {
                if now.duration_since(entry.last_seen) > PEER_TTL {
                    to_remove.push(id.clone());
                    false
                } else {
                    true
                }
            });
        }
        for id in to_remove {
            let _ = app.emit("synapse:peer-removed", serde_json::json!({ "id": id }));
        }
    }
}

fn pipe_output(app: &AppHandle, child: &mut Child) {
    if let Some(stdout) = child.stdout.take() {
        let app = app.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit(
                    "synapse:log",
                    serde_json::json!({ "stream": "stdout", "line": line }),
                );
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let app = app.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit(
                    "synapse:log",
                    serde_json::json!({ "stream": "stderr", "line": line }),
                );
            }
        });
    }
}
