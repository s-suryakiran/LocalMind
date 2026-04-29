// Phase 3 chunk A foundation; consumed by chunks B/C/E. See synapse_proto
// for matching note. Module-level allow keeps CI green during the chunked
// rollout — will be removed once the proxy + host pool actually call in.
#![allow(dead_code)]

// Persisted Synapse worker token. Generated on first call, then reused
// across app restarts so a paired host doesn't have to re-pair every time
// the worker reboots. The file lives at `<data_dir>/synapse-token.txt`.
//
// We keep this in its own module (vs inlining in synapse.rs) because both
// the worker auth proxy AND the host beacon-verifier need to read the local
// token, and centralizing here means there's exactly one place that knows
// the on-disk format. Future changes (rotation, multiple-token support)
// happen here.
use crate::config;
use crate::synapse_proto;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// In-process cache so repeated calls don't hit the filesystem. We don't
/// expect the token to change while the app is running — and if a future
/// "rotate token" feature lands, it'll explicitly invalidate this cache.
static CACHE: Mutex<Option<String>> = Mutex::new(None);

/// Returns the current token, creating + persisting one on first call.
/// Call freely; the lock is sub-microsecond once warm.
pub fn load_or_create() -> Result<String> {
    {
        let cache = CACHE.lock().unwrap();
        if let Some(t) = cache.as_ref() {
            return Ok(t.clone());
        }
    }

    let path = config::synapse_token_path();
    let token = match fs::read_to_string(&path) {
        Ok(s) => {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                // File got truncated somehow — regenerate rather than ship
                // an empty token through the auth proxy.
                let fresh = synapse_proto::generate_token();
                write_token(&path, &fresh)?;
                fresh
            } else {
                trimmed
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let fresh = synapse_proto::generate_token();
            write_token(&path, &fresh)?;
            fresh
        }
        Err(e) => return Err(e).context("reading synapse token file"),
    };

    *CACHE.lock().unwrap() = Some(token.clone());
    Ok(token)
}

/// Force a fresh token, replacing whatever's on disk. Returns the new value.
/// Existing host pairings will break until the user re-pastes the new token.
pub fn rotate() -> Result<String> {
    let path = config::synapse_token_path();
    let fresh = synapse_proto::generate_token();
    write_token(&path, &fresh)?;
    *CACHE.lock().unwrap() = Some(fresh.clone());
    Ok(fresh)
}

fn write_token(path: &PathBuf, token: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating token dir")?;
    }
    // We don't bother with restrictive perms (0600) because the file lives
    // under the user's data dir, which is already user-scoped on every
    // supported OS. Matching that with explicit chmod would be theatre.
    fs::write(path, format!("{token}\n")).context("writing token file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_stable_across_calls() {
        // Reset the cache so this test isn't order-dependent with others.
        *CACHE.lock().unwrap() = None;
        let a = load_or_create().unwrap();
        let b = load_or_create().unwrap();
        assert_eq!(a, b);
        assert!(a.len() >= 32, "token should be base32 256-bit-ish");
    }

    #[test]
    fn rotate_changes_token() {
        let _ = load_or_create();
        let before = load_or_create().unwrap();
        let after = rotate().unwrap();
        assert_ne!(before, after);
        let next = load_or_create().unwrap();
        assert_eq!(after, next);
    }
}
