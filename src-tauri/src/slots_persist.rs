//! Persisted "active slots" snapshot. Trivial schema — one model id
//! per role. We deliberately don't persist port numbers or PIDs;
//! those are recomputed at restore time.

use crate::config::slots_state_path;
use crate::slots::Role;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlotsSnapshot {
    pub chat_model_id: Option<String>,
    pub embed_model_id: Option<String>,
    pub vision_model_id: Option<String>,
    pub vision_mmproj_id: Option<String>,
}

pub fn load() -> SlotsSnapshot {
    let path = slots_state_path();
    let Ok(bytes) = fs::read(&path) else {
        return SlotsSnapshot::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save(snap: &SlotsSnapshot) -> Result<()> {
    let path = slots_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(snap)?;
    fs::write(&path, json)?;
    Ok(())
}

pub fn snapshot_from_status(slots: &[crate::slots::SlotStatus]) -> SlotsSnapshot {
    let mut by_role: HashMap<Role, &crate::slots::SlotStatus> = HashMap::new();
    for s in slots {
        if s.running {
            by_role.insert(s.role, s);
        }
    }
    SlotsSnapshot {
        chat_model_id: by_role.get(&Role::Chat).and_then(|s| s.model_id.clone()),
        embed_model_id: by_role.get(&Role::Embed).and_then(|s| s.model_id.clone()),
        vision_model_id: by_role.get(&Role::Vision).and_then(|s| s.model_id.clone()),
        vision_mmproj_id: by_role.get(&Role::Vision).and_then(|s| s.mmproj_id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slots::SlotStatus;

    #[test]
    fn snapshot_only_includes_running_slots() {
        let slots = vec![
            SlotStatus {
                role: Role::Chat,
                running: true,
                port: 8181,
                model_id: Some("chat-7b".into()),
                mmproj_id: None,
                pid: Some(123),
            },
            SlotStatus {
                role: Role::Embed,
                running: false,
                port: 8182,
                model_id: None,
                mmproj_id: None,
                pid: None,
            },
            SlotStatus {
                role: Role::Vision,
                running: true,
                port: 8183,
                model_id: Some("llava-7b".into()),
                mmproj_id: Some("llava-mmproj".into()),
                pid: Some(456),
            },
        ];
        let snap = snapshot_from_status(&slots);
        assert_eq!(snap.chat_model_id.as_deref(), Some("chat-7b"));
        assert_eq!(snap.embed_model_id, None);
        assert_eq!(snap.vision_model_id.as_deref(), Some("llava-7b"));
        assert_eq!(snap.vision_mmproj_id.as_deref(), Some("llava-mmproj"));
    }
}
