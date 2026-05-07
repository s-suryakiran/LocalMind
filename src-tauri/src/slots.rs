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
}
