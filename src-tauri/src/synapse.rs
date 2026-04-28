// Phase 2 Synapse: each machine can run an `rpc-server` child process that
// llama.cpp on a remote host reaches via `--rpc host:port`. On top of Phase 1's
// manual start/stop, we now also:
//   - advertise the worker on the LAN via mDNS (`_localmind-synapse._tcp`)
//   - browse for other workers, emitting peer add/remove events to the UI
//   - expose a `restart_worker` so the host can flush worker VRAM on demand
//
// Auth + smart layer split + tok/s telemetry come in Phase 3.
use crate::{binaries, config};
use anyhow::{anyhow, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

pub const DEFAULT_WORKER_PORT: u16 = 50052;
const SERVICE_TYPE: &str = "_localmind-synapse._tcp.local.";

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
}

struct WorkerHandle {
    child: Option<Child>,
    port: u16,
    /// Owns the mDNS advertisement; dropping unregisters automatically.
    advertised: Option<ServiceInfo>,
}

pub struct SynapseState {
    worker: Mutex<WorkerHandle>,
    /// Single shared mDNS daemon (advertise + browse share one socket).
    daemon: Mutex<Option<ServiceDaemon>>,
    /// Peers we've seen, keyed by mDNS instance name. Cached so the UI can
    /// re-render on demand without waiting for the next browse tick.
    peers: Mutex<HashMap<String, SynapsePeer>>,
}

impl SynapseState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            worker: Mutex::new(WorkerHandle {
                child: None,
                port: DEFAULT_WORKER_PORT,
                advertised: None,
            }),
            daemon: Mutex::new(None),
            peers: Mutex::new(HashMap::new()),
        })
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
        self.peers.lock().await.values().cloned().collect()
    }

    pub async fn stop_worker(&self) -> Result<()> {
        let mut w = self.worker.lock().await;
        if let Some(mut c) = w.child.take() {
            let _ = c.kill().await;
            let _ = c.wait().await;
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
        let port = port.unwrap_or(DEFAULT_WORKER_PORT);
        crate::llama::kill_orphan_on_port(port).await;

        let binary = config::rpc_server_path();
        if !binary.exists() {
            return Err(anyhow!(
                "rpc-server binary not found at {} — is the bundled llama.cpp build missing RPC support?",
                binary.display()
            ));
        }

        // Bind to 0.0.0.0 so other machines on the LAN can connect; the host
        // typed our address into its workers field. There's no auth on the RPC
        // wire (Phase 3 will fix that with an auth shim) — only run worker
        // mode on networks you control.
        let mut cmd = Command::new(&binary);
        cmd.arg("-H").arg("0.0.0.0");
        cmd.arg("-p").arg(port.to_string());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn rpc-server: {e}"))?;
        pipe_output(app, &mut child);

        // Advertise on mDNS so the host's Synapse page picks us up automatically.
        // Best-effort: if advertising fails (e.g. no multicast on this NIC, or
        // hostname has chars mdns-sd refuses) the worker still works, the host
        // just has to type the IP manually. Surface the failure as a synapse:log
        // line so the UI can show *why* discovery isn't happening.
        let advertised = match self.advertise(port).await {
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
                            port,
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

        {
            let mut w = self.worker.lock().await;
            w.child = Some(child);
            w.port = port;
            w.advertised = advertised;
        }

        let _ = app.emit("synapse:ready", serde_json::json!({ "port": port }));

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
        let app = app.clone();
        let me = self.clone();
        tokio::spawn(async move {
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
