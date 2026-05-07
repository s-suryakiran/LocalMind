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
}
