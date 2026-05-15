//! iOS-only Tauri command surface. All entries gated `#[cfg(target_os = "ios")]`
//! at the `lib.rs` module declaration; this file is never compiled on desktop.
//!
//! Populated by M2 (qr, mdns, keychain), M3 (biometric), M4 (lifecycle),
//! and M5 (push). M1 ships the empty gate so subsequent milestones start
//! on a known-good baseline.

/// Version string for the iOS bridge. Used by `ios_bridge_version` so the
/// React layer can assert it's running against the expected Rust build.
pub const IOS_BRIDGE_VERSION: &str = "1";

#[tauri::command]
pub fn ios_bridge_version() -> &'static str {
    IOS_BRIDGE_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_v1() {
        assert_eq!(ios_bridge_version(), "1");
    }
}
