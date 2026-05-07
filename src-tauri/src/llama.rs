use crate::{binaries, hardware, host_proxy::HostProxy, models};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// One Synapse worker the host wants to pipeline-shard layers onto. Phase 3
/// adds the token field — every authenticated worker has a unique token,
/// shown in its Synapse UI, that the host pastes here.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SynapseWorker {
    /// `host:port` of the worker's auth proxy (the public-facing port).
    pub endpoint: String,
    /// Base32-encoded token from the worker. Without it, the handshake is
    /// rejected and the model load fails fast at `start`.
    pub token: String,
    /// Phase 3 chunk G: relative compute weight (0.0–1.0). When any worker
    /// has a non-default weight, we pass `--tensor-split host,w1,w2,…` so
    /// llama.cpp distributes layers proportionally instead of evenly.
    /// `None` (or 0.0) means "use llama.cpp's default" — even split.
    #[serde(default)]
    pub weight: Option<f32>,
    /// Phase 4 chunk N: pinned cert fingerprint. The host's TLS verifier
    /// rejects any cert that doesn't match this hash, so a peer at the
    /// same IP can't impersonate the worker even if it learns the token.
    /// Captured by the host on first pairing from the (HMAC-verified)
    /// beacon. `None` means "fall back to first-seen TOFU on connect."
    #[serde(default)]
    pub cert_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlamaSettings {
    pub model_id: String,
    pub context_size: Option<u32>,
    pub n_gpu_layers: Option<i32>,
    pub threads: Option<u32>,
    pub port: Option<u16>,
    pub mmproj_id: Option<String>,
    pub flash_attn: Option<bool>,
    /// Synapse workers to pipeline-shard layers onto. For each entry we spin
    /// up a local proxy that handshakes with the worker using the supplied
    /// token; llama-server then connects to the local proxy address. The
    /// final `--rpc` arg is built from those local addresses, so the remote
    /// IPs/tokens never appear on the llama-server command line.
    pub synapse_workers: Option<Vec<SynapseWorker>>,
    /// Phase 3 chunk G: relative weight for the *host* device in the layer
    /// split. Combined with each worker's `weight` to build `--tensor-split`.
    /// `None` keeps llama.cpp's default (even split).
    #[serde(default)]
    pub host_weight: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlamaStatus {
    /// Legacy flat fields — derived from `slots`. Kept populated for
    /// one release so the frontend can migrate incrementally. New code
    /// should read `slots` directly.
    pub running: bool,
    pub port: u16,
    pub model_id: Option<String>,
    pub mmproj_id: Option<String>,
    pub pid: Option<u32>,
    pub embedding_running: bool,
    pub embedding_port: u16,
    pub embedding_model_id: Option<String>,
    /// Canonical: one entry per role (always exactly 3, in the order
    /// chat / embed / vision). `running == false` for unloaded roles.
    pub slots: Vec<crate::slots::SlotStatus>,
}

pub struct LlamaState {
    /// One mutex over the whole table — all slot transitions go through
    /// it. Slots are short-running operations (spawn child, wait_ready);
    /// the mutex isn't held while llama-server is generating.
    table: Mutex<crate::slots::SlotTable>,
    /// Phase 3: per-worker local proxies that authenticate with each
    /// remote worker before bytes flow. Lifecycle is tied to the **chat**
    /// slot specifically — pipeline-sharded inference only makes sense
    /// for the chat model. Embed and vision stay local-only.
    host_proxy: Arc<HostProxy>,
}

impl LlamaState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            table: Mutex::new(crate::slots::SlotTable::new()),
            host_proxy: HostProxy::new(),
        })
    }

    pub async fn status(&self) -> LlamaStatus {
        let table = self.table.lock().await;
        let chat = table.get(crate::slots::Role::Chat);
        let embed = table.get(crate::slots::Role::Embed);
        let slots = table.statuses();
        LlamaStatus {
            running: chat.is_some(),
            port: chat
                .map(|e| e.port)
                .unwrap_or_else(|| crate::slots::Role::Chat.default_port()),
            model_id: chat.map(|e| e.model_id.clone()),
            mmproj_id: chat.and_then(|e| e.mmproj_id.clone()),
            pid: chat.and_then(|e| e.child.id()),
            embedding_running: embed.is_some(),
            embedding_port: embed
                .map(|e| e.port)
                .unwrap_or_else(|| crate::slots::Role::Embed.default_port()),
            embedding_model_id: embed.map(|e| e.model_id.clone()),
            slots,
        }
    }

    pub async fn embedding_port(&self) -> u16 {
        self.table
            .lock()
            .await
            .get(crate::slots::Role::Embed)
            .map(|e| e.port)
            .unwrap_or_else(|| crate::slots::Role::Embed.default_port())
    }

    pub async fn embedding_running(&self) -> bool {
        self.table
            .lock()
            .await
            .get(crate::slots::Role::Embed)
            .is_some()
    }

    pub async fn stop(&self) -> Result<()> {
        if let Some(mut entry) = self
            .table
            .lock()
            .await
            .remove(crate::slots::Role::Chat)
        {
            let _ = entry.child.kill().await;
        }
        // Synapse host proxies are tied to the chat slot's lifecycle —
        // tear them down whenever chat goes away. Phase 3 invariant.
        self.host_proxy.stop_all().await;
        persist_snapshot(self).await;
        Ok(())
    }

    pub async fn stop_embedding(&self) -> Result<()> {
        if let Some(mut entry) = self
            .table
            .lock()
            .await
            .remove(crate::slots::Role::Embed)
        {
            let _ = entry.child.kill().await;
        }
        persist_snapshot(self).await;
        Ok(())
    }

    pub async fn start(&self, app: &AppHandle, settings: LlamaSettings) -> Result<LlamaStatus> {
        self.stop().await?;

        let model_size = std::fs::metadata(models::model_path(&settings.model_id)?)
            .map(|m| m.len())
            .unwrap_or(0);
        preflight_vram(self, crate::slots::Role::Chat, model_size).await?;

        let port = settings.port.unwrap_or(8181);
        kill_orphan_on_port(port).await;

        let binary = binaries::ensure_llama_server(app).await?;
        let model = models::model_path(&settings.model_id)?;
        let hw = hardware::detect();

        let ctx = settings.context_size.unwrap_or(4096);
        let n_gpu = settings.n_gpu_layers.unwrap_or(hw.recommended_n_gpu_layers);
        let threads = settings
            .threads
            .unwrap_or((hw.cpu_cores as u32).saturating_sub(1).max(1));

        let mut cmd = Command::new(&binary);
        cmd.arg("-m").arg(&model);
        cmd.arg("--host").arg("127.0.0.1");
        cmd.arg("--port").arg(port.to_string());
        cmd.arg("-c").arg(ctx.to_string());
        cmd.arg("-t").arg(threads.to_string());
        cmd.arg("-ngl").arg(n_gpu.to_string());
        cmd.arg("--jinja");
        cmd.arg("-fa").arg(if settings.flash_attn.unwrap_or(true) {
            "on"
        } else {
            "off"
        });
        // Synapse: pipeline-shard layers across authenticated remote workers.
        //
        // Phase 3 routes each worker through a local proxy that handshakes
        // with the worker's auth_proxy before any rpc bytes flow. We probe
        // the handshake here BEFORE spawning llama-server — if a token is
        // wrong or a worker is unreachable, the user sees a clear error
        // instead of a half-loaded model that can't infer.
        //
        // The local addresses we hand to llama-server look like
        // `127.0.0.1:54712`; the remote IPs/tokens never appear on the
        // command line, which keeps them out of `ps`/process listings and
        // any third-party debug tooling that walks argv.
        if let Some(workers) = &settings.synapse_workers {
            let mut local_addrs: Vec<String> = Vec::new();
            // Track only the workers we successfully started so the
            // tensor-split list lines up with --rpc devices exactly.
            let mut active_workers: Vec<&SynapseWorker> = Vec::new();
            for w in workers {
                let endpoint = w.endpoint.trim();
                if endpoint.is_empty() {
                    continue;
                }
                match self
                    .host_proxy
                    .start(app, endpoint, &w.token, w.cert_fingerprint.as_deref())
                    .await
                {
                    Ok(local) => {
                        local_addrs.push(local);
                        active_workers.push(w);
                    }
                    Err(e) => {
                        // One bad worker shouldn't strand proxies for the
                        // good ones — tear them all down before bailing so
                        // the user gets back to a clean slate.
                        self.host_proxy.stop_all().await;
                        return Err(anyhow!("synapse worker {endpoint}: {e}"));
                    }
                }
            }
            if !local_addrs.is_empty() {
                cmd.arg("--rpc").arg(local_addrs.join(","));

                // Phase 3 chunk G: build --tensor-split if the user set any
                // explicit weights. The list goes [host, w1, w2, …] in the
                // SAME order llama.cpp sees `--rpc` devices. Skipping the
                // arg entirely (when no weights provided) is intentional —
                // that lets llama.cpp use its default split heuristic
                // instead of forcing a probably-wrong number we'd have to
                // synthesize.
                let any_explicit = settings.host_weight.is_some()
                    || active_workers.iter().any(|w| w.weight.is_some());
                if any_explicit {
                    let mut weights: Vec<f32> = Vec::with_capacity(active_workers.len() + 1);
                    // Default 0.5 matches the UI slider's visual resting
                    // position. `unwrap_or(1.0)` made an untouched worker
                    // silently outweigh a host the user had dialed back to
                    // a low value, which inverted the layer split versus
                    // what the sliders showed.
                    weights.push(settings.host_weight.unwrap_or(0.5).max(0.0));
                    for w in &active_workers {
                        weights.push(w.weight.unwrap_or(0.5).max(0.0));
                    }
                    // Normalize so the user can think in percentages without
                    // worrying about the sum. llama.cpp accepts comma-
                    // separated floats; we render with 3 decimals.
                    let sum: f32 = weights.iter().sum();
                    if sum > 0.0 {
                        let csv = weights
                            .iter()
                            .map(|w| format!("{:.3}", w / sum))
                            .collect::<Vec<_>>()
                            .join(",");
                        cmd.arg("--tensor-split").arg(csv);
                    }
                }
            }
        }

        let mut loaded_mmproj: Option<String> = None;
        if let Some(mmproj_id) = &settings.mmproj_id {
            if let Ok(p) = models::model_path(mmproj_id) {
                cmd.arg("--mmproj").arg(p);
                loaded_mmproj = Some(mmproj_id.clone());
            }
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn llama-server: {e}"))?;
        pipe_output(app, &mut child, "chat");

        {
            let mut table = self.table.lock().await;
            // Insert atomically — if a previous chat slot existed (it shouldn't
            // after the `self.stop().await?` at the top, but defensively):
            if let Some(mut prev) = table.insert(
                crate::slots::Role::Chat,
                crate::slots::SlotEntry {
                    child,
                    port,
                    model_id: settings.model_id.clone(),
                    mmproj_id: loaded_mmproj.clone(),
                },
            ) {
                let _ = prev.child.kill().await;
            }
        }

        wait_ready(port).await?;

        let _ = app.emit(
            "llama:ready",
            serde_json::json!({ "port": port, "modelId": settings.model_id, "stream": "chat" }),
        );

        persist_snapshot(self).await;
        Ok(self.status().await)
    }

    pub async fn start_embedding(&self, app: &AppHandle, model_id: String) -> Result<LlamaStatus> {
        self.stop_embedding().await?;

        let model_size = std::fs::metadata(models::model_path(&model_id)?)
            .map(|m| m.len())
            .unwrap_or(0);
        preflight_vram(self, crate::slots::Role::Embed, model_size).await?;

        let port = 8182;
        kill_orphan_on_port(port).await;

        let binary = binaries::ensure_llama_server(app).await?;
        let model = models::model_path(&model_id)?;
        let hw = hardware::detect();
        let threads = (hw.cpu_cores as u32).saturating_sub(1).max(1);
        let n_gpu = hw.recommended_n_gpu_layers;

        let mut cmd = Command::new(&binary);
        cmd.arg("-m").arg(&model);
        cmd.arg("--host").arg("127.0.0.1");
        cmd.arg("--port").arg(port.to_string());
        cmd.arg("-t").arg(threads.to_string());
        cmd.arg("-ngl").arg(n_gpu.to_string());
        cmd.arg("--embeddings");
        cmd.arg("--pooling").arg("mean");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn embedding server: {e}"))?;
        pipe_output(app, &mut child, "embed");

        {
            let mut table = self.table.lock().await;
            if let Some(mut prev) = table.insert(
                crate::slots::Role::Embed,
                crate::slots::SlotEntry {
                    child,
                    port,
                    model_id: model_id.clone(),
                    mmproj_id: None,
                },
            ) {
                let _ = prev.child.kill().await;
            }
        }

        wait_ready(port).await?;

        let _ = app.emit(
            "llama:ready",
            serde_json::json!({ "port": port, "modelId": model_id, "stream": "embed" }),
        );

        persist_snapshot(self).await;
        Ok(self.status().await)
    }

    /// Start a dedicated vision slot. Independent of the chat slot:
    /// a user can have a 7B chat model + a 7B LLaVA loaded at the same
    /// time, and image-bearing chat requests get routed to this slot.
    /// Synapse pipelining is intentionally NOT applied — vision is
    /// local-only.
    pub async fn start_vision(
        &self,
        app: &AppHandle,
        model_id: String,
        mmproj_id: String,
    ) -> Result<LlamaStatus> {
        // Stop any existing vision slot first — we only support one at a time.
        if let Some(mut prev) = self
            .table
            .lock()
            .await
            .remove(crate::slots::Role::Vision)
        {
            let _ = prev.child.kill().await;
        }

        let model_size = std::fs::metadata(models::model_path(&model_id)?)
            .map(|m| m.len())
            .unwrap_or(0);
        preflight_vram(self, crate::slots::Role::Vision, model_size).await?;

        let preferred = crate::slots::Role::Vision.default_port();
        kill_orphan_on_port(preferred).await;
        let pool = crate::slots::port_pool(crate::slots::Role::Vision);
        let port = crate::slots::pick_port(preferred, &pool, crate::slots::port_is_free)
            .ok_or_else(|| anyhow!("no free port in vision pool {pool:?}"))?;

        let binary = binaries::ensure_llama_server(app).await?;
        let model = models::model_path(&model_id)?;
        let mmproj_path = models::model_path(&mmproj_id)?;
        let hw = hardware::detect();
        let threads = (hw.cpu_cores as u32).saturating_sub(1).max(1);
        let n_gpu = hw.recommended_n_gpu_layers;

        let mut cmd = Command::new(&binary);
        cmd.arg("-m").arg(&model);
        cmd.arg("--mmproj").arg(&mmproj_path);
        cmd.arg("--host").arg("127.0.0.1");
        cmd.arg("--port").arg(port.to_string());
        cmd.arg("-t").arg(threads.to_string());
        cmd.arg("-ngl").arg(n_gpu.to_string());
        cmd.arg("--jinja");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn vision llama-server: {e}"))?;
        pipe_output(app, &mut child, "vision");

        {
            let mut table = self.table.lock().await;
            table.insert(
                crate::slots::Role::Vision,
                crate::slots::SlotEntry {
                    child,
                    port,
                    model_id: model_id.clone(),
                    mmproj_id: Some(mmproj_id.clone()),
                },
            );
        }

        wait_ready(port).await?;

        let _ = app.emit(
            "llama:ready",
            serde_json::json!({ "port": port, "modelId": model_id, "stream": "vision" }),
        );

        persist_snapshot(self).await;
        Ok(self.status().await)
    }

    pub async fn stop_vision(&self) -> Result<()> {
        if let Some(mut entry) = self
            .table
            .lock()
            .await
            .remove(crate::slots::Role::Vision)
        {
            let _ = entry.child.kill().await;
        }
        persist_snapshot(self).await;
        Ok(())
    }

    /// Read-only snapshot of the vision slot's port + model. None when
    /// vision isn't loaded. Used by `server.rs::proxy_v1` to route
    /// image-bearing requests.
    pub async fn vision_slot(&self) -> Option<(u16, String)> {
        self.table
            .lock()
            .await
            .get(crate::slots::Role::Vision)
            .map(|e| (e.port, e.model_id.clone()))
    }

    /// Pick the port a /v1/chat/completions request should be proxied to,
    /// based on whether the body has any image_url content parts. Vision
    /// slot wins for image-bearing requests when loaded; otherwise chat.
    /// Returns `None` if neither slot can serve.
    pub async fn route_chat_port(&self, has_image: bool) -> Option<u16> {
        let table = self.table.lock().await;
        if has_image {
            if let Some(e) = table.get(crate::slots::Role::Vision) {
                return Some(e.port);
            }
        }
        table.get(crate::slots::Role::Chat).map(|e| e.port)
    }
}

/// Pre-flight check: would loading `new_role` with the given file size
/// exceed available VRAM, given what's already loaded? Returns Ok if
/// fine, or Err with a user-readable message. CPU-only hardware skips
/// the check and returns Ok.
async fn preflight_vram(
    state: &LlamaState,
    new_role: crate::slots::Role,
    new_size_bytes: u64,
) -> Result<()> {
    let hw = hardware::detect();
    let Some(avail) = crate::slots::available_gpu_memory_gb(&hw) else {
        return Ok(());
    };
    let table = state.table.lock().await;
    let mut existing: Vec<u64> = Vec::new();
    for r in [
        crate::slots::Role::Chat,
        crate::slots::Role::Embed,
        crate::slots::Role::Vision,
    ] {
        // Skip the role we're about to (re)load — we'll free its
        // memory before spawning the new child.
        if r == new_role {
            continue;
        }
        if let Some(entry) = table.get(r) {
            if let Ok(p) = models::model_path(&entry.model_id) {
                if let Ok(meta) = std::fs::metadata(p) {
                    existing.push(meta.len());
                }
            }
        }
    }
    drop(table);
    if !crate::slots::fits_with_existing(&existing, new_size_bytes, avail) {
        return Err(anyhow!(
            "loading this model would exceed available GPU memory ({avail:.1} GB). Unload another slot first."
        ));
    }
    Ok(())
}

/// Snapshot the active slot set to disk so a relaunch can re-hydrate.
/// Failures are logged but never returned — persistence is a
/// nice-to-have, not a correctness requirement.
async fn persist_snapshot(state: &LlamaState) {
    let status = state.status().await;
    let snap = crate::slots_persist::snapshot_from_status(&status.slots);
    if let Err(e) = crate::slots_persist::save(&snap) {
        eprintln!("slots persist failed: {e}");
    }
}

fn pipe_output(app: &AppHandle, child: &mut Child, tag: &str) {
    if let Some(stdout) = child.stdout.take() {
        let app = app.clone();
        let tag = tag.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit(
                    "llama:log",
                    serde_json::json!({ "stream": "stdout", "line": line, "tag": tag }),
                );
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let app = app.clone();
        let tag = tag.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            // Phase 4 chunk Q: cluster-viz layout state. Collect device-
            // buffer lines as they stream and flush a single layout event
            // when llama-server signals load completion. One render
            // instead of N flickering ones.
            let mut device_buffers: Vec<(String, u64)> = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                // Phase 3 chunk F1: tok/s parser. llama.cpp's per-request
                // timing block looks like:
                //
                //   eval time = 4321.12 ms /   123 runs   (   34.95 ms per token,    28.62 tokens per second)
                //
                // We only care about the chat server (tag=="chat"), and only
                // about the *eval* line (generation throughput) — `prompt
                // eval time` is for prefill, which is interesting but
                // hugely variable depending on context length.
                if tag == "chat" {
                    if let Some(tok_per_sec) = parse_eval_tok_per_sec(&line) {
                        let _ = app.emit(
                            "synapse:metrics",
                            serde_json::json!({
                                "kind": "host-tok-s",
                                "tokPerSec": tok_per_sec,
                                "ts": now_ms(),
                            }),
                        );
                    }
                    if let Some((device, mb)) = parse_buffer_line(&line) {
                        device_buffers.push((device, mb));
                    }
                    // Heralds load-complete: emit one layout event with
                    // every device-buffer we collected, then clear so a
                    // model reload starts fresh.
                    if !device_buffers.is_empty()
                        && (line.contains("model loaded")
                            || line.contains("loaded successfully")
                            || line.contains("HTTP server listening")
                            || line.contains("starting the main loop"))
                    {
                        let _ = app.emit(
                            "synapse:cluster-layout",
                            serde_json::json!({
                                "devices": device_buffers
                                    .iter()
                                    .map(|(d, mb)| serde_json::json!({ "device": d, "mb": mb }))
                                    .collect::<Vec<_>>(),
                                "ts": now_ms(),
                            }),
                        );
                        device_buffers.clear();
                    }
                }
                let _ = app.emit(
                    "llama:log",
                    serde_json::json!({ "stream": "stderr", "line": line, "tag": tag }),
                );
            }
        });
    }
}

/// Phase 4 chunk Q: extract a (device, MiB) pair from llama.cpp's tensor-
/// loading log. Real lines look like:
///
///     load_tensors: CUDA0 model buffer size = 12345.67 MiB
///     load_tensors: CPU model buffer size  =   234.56 MiB
///     load_tensors: RPC[127.0.0.1:54712] model buffer size = 9876.54 MiB
///
/// Returns None for unrelated lines. We deliberately key off "model buffer
/// size" rather than just "buffer size" because llama.cpp also prints
/// transient KV-cache sizes that aren't part of the layout snapshot.
fn parse_buffer_line(line: &str) -> Option<(String, u64)> {
    if !line.contains("model buffer size") {
        return None;
    }
    let after_prefix = line.split_once("load_tensors:").map(|(_, r)| r)?;
    let (device_part, size_part) = after_prefix.split_once("model buffer size")?;
    let device = device_part.trim().to_string();
    if device.is_empty() {
        return None;
    }
    let num: String = size_part
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mb: f64 = num.parse().ok()?;
    Some((device, mb as u64))
}

/// Extract `tokens per second` from an llama.cpp eval-time log line, e.g.
/// `eval time = 4321.12 ms / 123 runs ( 34.95 ms per token, 28.62 tokens per second)`.
/// Returns `None` if the line isn't an eval-time block. We only match `eval time`
/// (post-decode generation), not `prompt eval time` which is one-shot prefill.
fn parse_eval_tok_per_sec(line: &str) -> Option<f64> {
    // Be permissive about whitespace; llama.cpp's spacing has shifted
    // across versions. We require the literal "eval time" prefix and the
    // closing "tokens per second" tail with a parsed float between them.
    let trimmed = line.trim_start();
    // "prompt eval time" lines start with "prompt", which we want to skip.
    if !trimmed.starts_with("eval time") {
        return None;
    }
    let lower = line.to_ascii_lowercase();
    let tail = lower.split("tokens per second").next()?;
    // Walk back from the tail to find the last comma-separated number.
    let chunk = tail.rsplit(',').next()?.trim();
    let num: String = chunk
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse::<f64>().ok()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) async fn kill_orphan_on_port(port: u16) {
    #[cfg(target_family = "unix")]
    {
        let out = tokio::process::Command::new("lsof")
            .arg("-t")
            .arg(format!("-iTCP:{}", port))
            .arg("-sTCP:LISTEN")
            .output()
            .await;
        if let Ok(o) = out {
            for pid in String::from_utf8_lossy(&o.stdout).lines() {
                let pid = pid.trim();
                if pid.is_empty() {
                    continue;
                }
                let _ = tokio::process::Command::new("kill")
                    .arg("-9")
                    .arg(pid)
                    .output()
                    .await;
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(out) = tokio::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Get-NetTCPConnection -LocalPort {} -ErrorAction SilentlyContinue | Select-Object -Expand OwningProcess",
                    port
                ),
            ])
            .output()
            .await
        {
            for pid in String::from_utf8_lossy(&out.stdout).lines() {
                let pid = pid.trim();
                if pid.is_empty() { continue; }
                let _ = tokio::process::Command::new("taskkill")
                    .args(["/F", "/PID", pid])
                    .output()
                    .await;
            }
        }
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

async fn wait_ready(port: u16) -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/health", port);
    // Allow up to ~3 minutes: first-time Metal shader compilation can take 10s+,
    // and large models may take additional time to mmap and warm up.
    for _ in 0..360 {
        if let Ok(r) = client.get(&url).send().await {
            if r.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(anyhow!(
        "llama-server did not become ready on port {port} within 3 min"
    ))
}

#[cfg(test)]
mod tests {
    use super::{parse_buffer_line, parse_eval_tok_per_sec};

    #[test]
    fn parses_buffer_lines_real_format() {
        let cuda = "load_tensors: CUDA0 model buffer size = 12345.67 MiB";
        assert_eq!(parse_buffer_line(cuda), Some(("CUDA0".to_string(), 12345)));

        let cpu = "load_tensors: CPU model buffer size  =   234.56 MiB";
        assert_eq!(parse_buffer_line(cpu), Some(("CPU".to_string(), 234)));

        let rpc = "load_tensors: RPC[127.0.0.1:54712] model buffer size = 9876.54 MiB";
        assert_eq!(
            parse_buffer_line(rpc),
            Some(("RPC[127.0.0.1:54712]".to_string(), 9876))
        );
    }

    #[test]
    fn rejects_kv_cache_and_unrelated_lines() {
        assert_eq!(
            parse_buffer_line("llama_kv_cache: CUDA0 KV buffer size = 256 MiB"),
            None
        );
        assert_eq!(parse_buffer_line("loading model"), None);
        assert_eq!(parse_buffer_line(""), None);
    }

    #[test]
    fn parses_eval_tok_per_sec_real_format() {
        // Real llama.cpp output, indented (cmake build) and not.
        let line = "eval time =    4321.12 ms /   123 runs   (   34.95 ms per token,    28.62 tokens per second)";
        assert!((parse_eval_tok_per_sec(line).unwrap() - 28.62).abs() < 1e-6);

        let indented = "        eval time = 100 ms /  10 runs   ( 10.0 ms per token,   100.00 tokens per second)";
        assert!((parse_eval_tok_per_sec(indented).unwrap() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn skips_prompt_eval_lines() {
        // Prompt eval is prefill, not generation throughput; we ignore.
        let line =
            "prompt eval time = 200 ms /  50 runs   (  4.0 ms per token, 250.00 tokens per second)";
        assert_eq!(parse_eval_tok_per_sec(line), None);
    }

    #[test]
    fn rejects_unrelated_lines() {
        assert_eq!(parse_eval_tok_per_sec("loading model"), None);
        assert_eq!(parse_eval_tok_per_sec(""), None);
        assert_eq!(parse_eval_tok_per_sec("eval time = nope"), None);
    }
}
