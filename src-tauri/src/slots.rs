//! Concurrent inference slots.
//!
//! A "slot" is a running `llama-server` child for a specific role:
//! Chat, Embed, or Vision. Each slot has its own port, model, and
//! lifecycle. Today llama.rs hard-coded two slots and tied vision to
//! the chat slot via `--mmproj`; this module generalises that.

use serde::{Deserialize, Serialize};

/// The roles a llama-server child can fulfil. The string form (the
/// `serde` tag) is the wire format used by Tauri commands and the
/// persisted slots.json file — do not rename without a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Chat,
    Embed,
    Vision,
}

impl Role {
    pub fn default_port(self) -> u16 {
        match self {
            Role::Chat => 8181,
            Role::Embed => 8182,
            Role::Vision => 8183,
        }
    }

    /// Stable string form for logs and Tauri-event payloads. Currently
    /// only exercised in tests; intentionally part of the public API
    /// because subsequent plans wire it into Tauri events.
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Chat => "chat",
            Role::Embed => "embed",
            Role::Vision => "vision",
        }
    }
}

/// Internal handle for a running llama-server child. Held inside the
/// SlotTable; not exposed across the module boundary because callers
/// have no business poking at the `Child` directly.
pub struct SlotEntry {
    pub child: tokio::process::Child,
    pub port: u16,
    pub model_id: String,
    pub mmproj_id: Option<String>,
}

/// Map keyed by Role. We don't use a HashMap — there are exactly three
/// roles and a fixed-size array is faster, simpler, and self-documents.
#[derive(Default)]
pub struct SlotTable {
    chat: Option<SlotEntry>,
    embed: Option<SlotEntry>,
    vision: Option<SlotEntry>,
}

impl SlotTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, role: Role) -> Option<&SlotEntry> {
        match role {
            Role::Chat => self.chat.as_ref(),
            Role::Embed => self.embed.as_ref(),
            Role::Vision => self.vision.as_ref(),
        }
    }

    pub fn insert(&mut self, role: Role, entry: SlotEntry) -> Option<SlotEntry> {
        let slot = match role {
            Role::Chat => &mut self.chat,
            Role::Embed => &mut self.embed,
            Role::Vision => &mut self.vision,
        };
        slot.replace(entry)
    }

    pub fn remove(&mut self, role: Role) -> Option<SlotEntry> {
        match role {
            Role::Chat => self.chat.take(),
            Role::Embed => self.embed.take(),
            Role::Vision => self.vision.take(),
        }
    }

    pub fn statuses(&self) -> Vec<SlotStatus> {
        [Role::Chat, Role::Embed, Role::Vision]
            .iter()
            .map(|&r| {
                let entry = self.get(r);
                SlotStatus {
                    role: r,
                    running: entry.is_some(),
                    port: entry.map(|e| e.port).unwrap_or_else(|| r.default_port()),
                    model_id: entry.map(|e| e.model_id.clone()),
                    mmproj_id: entry.and_then(|e| e.mmproj_id.clone()),
                    pid: entry.and_then(|e| e.child.id()),
                }
            })
            .collect()
    }
}

/// Pick a free port. Tries `preferred` first, then walks `pool` in order.
/// `is_free(port)` returns true when the port is bindable.
///
/// Pure over the probe so we can unit-test without binding sockets;
/// the production caller passes a probe that does a real `TcpListener::bind`.
pub fn pick_port<F: Fn(u16) -> bool>(preferred: u16, pool: &[u16], is_free: F) -> Option<u16> {
    if is_free(preferred) {
        return Some(preferred);
    }
    pool.iter().copied().find(|&p| p != preferred && is_free(p))
}

/// Production probe: try to bind, drop the listener immediately.
pub fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Default pool per role. We pick small, contiguous ranges so a curious
/// user `lsof`-ing the desktop sees obvious neighbours.
pub fn port_pool(role: Role) -> Vec<u16> {
    match role {
        Role::Chat => vec![8181, 8184, 8185, 8186],
        Role::Embed => vec![8182, 8187, 8188, 8189],
        Role::Vision => vec![8183, 8190, 8191, 8192],
    }
}

use crate::hardware::{Accelerator, HardwareInfo};

/// VRAM (or unified memory) available for inference, in GB. Returns
/// None for CPU-only systems (no hard upper bound — caller falls back
/// to RAM check). Apple Silicon reserves 30% for the OS and Metal
/// surfaces; on dedicated GPUs we use the full reported VRAM since the
/// OS lives in system RAM.
pub fn available_gpu_memory_gb(hw: &HardwareInfo) -> Option<f64> {
    match &hw.accelerator {
        Accelerator::AppleSilicon { unified_memory_gb, .. } => Some(unified_memory_gb * 0.7),
        Accelerator::Nvidia { vram_gb, .. } => Some(*vram_gb),
        Accelerator::Amd { vram_gb, .. } => Some(*vram_gb),
        Accelerator::IntelArc { .. } | Accelerator::Cpu => None,
    }
}

/// Rough per-slot VRAM estimate. A fully-offloaded GGUF takes
/// approximately its file size in VRAM at load time, plus ~0.5 GB
/// for KV cache + activation overhead at the default 4k context.
/// Conservative — we'd rather refuse a load that would have just
/// barely fit than OOM mid-inference.
pub fn estimate_slot_vram_gb(file_size_bytes: u64) -> f64 {
    (file_size_bytes as f64 / 1024.0 / 1024.0 / 1024.0) + 0.5
}

/// Would loading a new model with `new_size_bytes` push total estimated
/// VRAM use past `available_gb`?
pub fn fits_with_existing(
    existing_sizes: &[u64],
    new_size_bytes: u64,
    available_gb: f64,
) -> bool {
    let used: f64 = existing_sizes.iter().map(|&b| estimate_slot_vram_gb(b)).sum();
    let needed = estimate_slot_vram_gb(new_size_bytes);
    used + needed <= available_gb
}

/// One slot's externally-visible state. Returned to the frontend as
/// part of `LlamaStatus.slots`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotStatus {
    pub role: Role,
    pub running: bool,
    pub port: u16,
    pub model_id: Option<String>,
    /// Vision slot only — the matching projector model id. None for
    /// other roles or when vision is loaded without a separate mmproj.
    pub mmproj_id: Option<String>,
    pub pid: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_default_ports_are_distinct() {
        assert_ne!(Role::Chat.default_port(), Role::Embed.default_port());
        assert_ne!(Role::Chat.default_port(), Role::Vision.default_port());
        assert_ne!(Role::Embed.default_port(), Role::Vision.default_port());
    }

    #[test]
    fn role_serializes_kebab_case() {
        let json = serde_json::to_string(&Role::Vision).unwrap();
        assert_eq!(json, "\"vision\"");
    }

    #[test]
    fn role_as_str_matches_serde_tag() {
        assert_eq!(Role::Chat.as_str(), "chat");
        assert_eq!(Role::Embed.as_str(), "embed");
        assert_eq!(Role::Vision.as_str(), "vision");
    }

    use std::process::Stdio;

    fn dummy_child() -> tokio::process::Child {
        // Spawn `true` (Unix) or `cmd /c` (Windows) — we just need a Child
        // we can hold and drop without it actually doing anything.
        #[cfg(target_family = "unix")]
        {
            tokio::process::Command::new("true").stdout(Stdio::null()).spawn().unwrap()
        }
        #[cfg(target_os = "windows")]
        {
            tokio::process::Command::new("cmd").args(["/c", "exit"]).stdout(Stdio::null()).spawn().unwrap()
        }
    }

    fn dummy_entry(port: u16, model_id: &str) -> SlotEntry {
        SlotEntry {
            child: dummy_child(),
            port,
            model_id: model_id.to_string(),
            mmproj_id: None,
        }
    }

    #[test]
    fn empty_table_has_no_running_slots() {
        let t = SlotTable::new();
        let statuses = t.statuses();
        assert_eq!(statuses.len(), 3);
        assert!(statuses.iter().all(|s| !s.running));
    }

    // The remaining tests need a tokio runtime because they spawn
    // child processes via `tokio::process::Command` (so SlotEntry can
    // hold a real `tokio::process::Child`).

    #[tokio::test]
    async fn insert_then_get_returns_entry() {
        let mut t = SlotTable::new();
        t.insert(Role::Chat, dummy_entry(8181, "chat-7b"));
        assert!(t.get(Role::Chat).is_some());
        assert!(t.get(Role::Embed).is_none());
    }

    #[tokio::test]
    async fn insert_returns_previous_entry() {
        let mut t = SlotTable::new();
        t.insert(Role::Chat, dummy_entry(8181, "chat-7b"));
        let prev = t.insert(Role::Chat, dummy_entry(8181, "chat-13b"));
        assert!(prev.is_some());
        assert_eq!(prev.unwrap().model_id, "chat-7b");
        assert_eq!(t.get(Role::Chat).unwrap().model_id, "chat-13b");
    }

    #[tokio::test]
    async fn remove_takes_entry_out() {
        let mut t = SlotTable::new();
        t.insert(Role::Vision, dummy_entry(8183, "llava"));
        let taken = t.remove(Role::Vision);
        assert!(taken.is_some());
        assert!(t.get(Role::Vision).is_none());
    }

    #[tokio::test]
    async fn statuses_reflects_loaded_slots() {
        let mut t = SlotTable::new();
        t.insert(Role::Embed, dummy_entry(8182, "nomic-embed"));
        let statuses = t.statuses();
        let embed = statuses.iter().find(|s| s.role == Role::Embed).unwrap();
        assert!(embed.running);
        assert_eq!(embed.model_id.as_deref(), Some("nomic-embed"));
    }

    #[test]
    fn allocator_returns_preferred_when_free() {
        let probe = |_: u16| true;
        let port = pick_port(8181, &[8181, 8190], probe);
        assert_eq!(port, Some(8181));
    }

    #[test]
    fn allocator_skips_busy_preferred() {
        let probe = |p: u16| p != 8181;
        let port = pick_port(8181, &[8181, 8182], probe);
        assert_eq!(port, Some(8182));
    }

    #[test]
    fn allocator_returns_none_when_all_busy() {
        let probe = |_: u16| false;
        let port = pick_port(8181, &[8181, 8182, 8183], probe);
        assert_eq!(port, None);
    }

    fn hw(accel: Accelerator) -> HardwareInfo {
        HardwareInfo {
            os: "macos".into(),
            arch: "aarch64".into(),
            cpu_name: "test".into(),
            cpu_cores: 8,
            total_memory_gb: 16.0,
            accelerator: accel,
            recommended_backend: "metal".into(),
            recommended_n_gpu_layers: -1,
        }
    }

    #[test]
    fn available_gpu_memory_apple_silicon_reserves_30pct() {
        let info = hw(Accelerator::AppleSilicon { chip: "M1 Pro".into(), unified_memory_gb: 16.0 });
        let avail = available_gpu_memory_gb(&info).unwrap();
        // 70% of 16 = 11.2; allow small float wiggle.
        assert!((avail - 11.2).abs() < 0.01);
    }

    #[test]
    fn available_gpu_memory_nvidia_uses_full_vram() {
        let info = hw(Accelerator::Nvidia { name: "4090".into(), vram_gb: 24.0, cuda_version: None });
        assert_eq!(available_gpu_memory_gb(&info), Some(24.0));
    }

    #[test]
    fn available_gpu_memory_cpu_returns_none() {
        let info = hw(Accelerator::Cpu);
        assert_eq!(available_gpu_memory_gb(&info), None);
    }

    #[test]
    fn slot_vram_estimate_adds_kv_overhead() {
        // 4 GB file → ~4.5 GB estimate (file + 0.5 GB for KV cache).
        let bytes = 4 * 1024 * 1024 * 1024_u64;
        let est = estimate_slot_vram_gb(bytes);
        assert!((est - 4.5).abs() < 0.01);
    }

    #[test]
    fn fits_with_existing_passes_when_under_budget() {
        // 4 GB existing + 2 GB new = ~7 GB used (incl. overhead) within 16 GB.
        let existing = vec![4 * 1024 * 1024 * 1024_u64];
        let new_model = 2 * 1024 * 1024 * 1024_u64;
        assert!(fits_with_existing(&existing, new_model, 16.0));
    }

    #[test]
    fn fits_with_existing_fails_when_over_budget() {
        // 10 GB + 8 GB ≈ 19 GB > 16 GB available.
        let existing = vec![10 * 1024 * 1024 * 1024_u64];
        let new_model = 8 * 1024 * 1024 * 1024_u64;
        assert!(!fits_with_existing(&existing, new_model, 16.0));
    }
}
