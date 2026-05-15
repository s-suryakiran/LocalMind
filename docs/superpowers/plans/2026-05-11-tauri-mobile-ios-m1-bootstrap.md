# Tauri Mobile iOS — Milestone 1: Bootstrap & Build

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Get a locally-signed iOS build of LocalMind onto a real iPhone, hosting the existing React app inside WKWebView with no functional regressions on desktop.

**Architecture:** Use the Tauri 2 mobile scaffolding already in `src-tauri/gen/apple/`. Add an `iOS` section to `tauri.conf.json`, layer in iOS-specific `Info.plist` entries (mic, camera, local network, ATS), create an iOS-only capability allow-list, scaffold a cfg-gated Rust module for future iOS-only commands, and validate the build chain end-to-end. No new product features in M1 — this milestone exists to de-risk the iOS pipeline before M2 (pairing & discovery) starts.

**Tech Stack:** Tauri 2.x mobile (iOS target), Xcode 16+, Swift 5.9+, Rust (existing `src-tauri/` crate, `target_os = "ios"` cfg gates), React 19 (existing, unchanged), **free Apple ID with Xcode "Personal Team" signing** (no paid Apple Developer Program for M1–M4).

**Cost-deferral path (decided 2026-05-11):** This plan ships using free Xcode signing rather than a $99/yr Apple Developer Program account. Trade-offs accepted:
- App expires every 7 days; reinstall via `npm run tauri ios dev` when it does.
- Max 3 free-signed apps per device at a time.
- No TestFlight, no App Store, no APNs push, no App Groups (share extension). The first three only become required at M5 (push) and M6 (App Store); the share extension belongs to M3 and will need an alternative IPC mechanism that doesn't require App Groups, OR will be deferred until you pay the $99 then.

**Spec:** [`docs/superpowers/specs/2026-05-11-tauri-mobile-ios-parity-design.md`](../specs/2026-05-11-tauri-mobile-ios-parity-design.md), §5 ("iOS Surfaces & Build Pipeline") and §8 ("Phased Milestones") M1.

**M2–M6 plans:** Not in scope here. Each subsequent milestone gets its own plan written when M1 ships, so the plan reflects the actual rough edges discovered during M1.

---

## File Structure

### Created

- `src-tauri/Info.ios.plist` — iOS-only Info.plist key additions, merged by Tauri at build time
- `src-tauri/capabilities/ios.json` — iOS-only Tauri capability allow-list
- `src-tauri/src/ios/mod.rs` — iOS-only Rust module gate (empty in M1; populated by M2+)
- `src-tauri/tauri.ios.local.conf.json` — **gitignored** per-developer Tauri config override that injects the free-signing Team ID at build time. Each contributor creates their own (see `docs/ios-developer-setup.md`); never committed because (a) the repo is public and (b) each contributor has their own free Team ID.
- `docs/ios-developer-setup.md` — written-down setup checklist (Xcode install, Apple ID sign-in, Team ID extraction, local override file creation, device trust) so anyone joining the project can do this once
- `.github/workflows/ios.yml` — GitHub Actions workflow *(DEFERRED until paid Apple Developer enrollment; documented in Task 10)*

### Modified

- `src-tauri/tauri.conf.json` — add `bundle.iOS` section (placeholder development team `REPLACE_WITH_YOUR_TEAM_ID`, min iOS version 15.0). The real Team ID is supplied per-developer via the gitignored `tauri.ios.local.conf.json` override.
- `src-tauri/src/lib.rs` — wire `#[cfg(target_os = "ios")] mod ios;`
- `package.json` — convenience scripts (`ios:dev`, `ios:build`) that pass `--config src-tauri/tauri.ios.local.conf.json` to merge in the per-developer Team ID at build time
- `README.md` — extend the "Going fully native (Tauri Mobile)" section with concrete commands and prerequisites, link to `docs/ios-developer-setup.md`
- `.gitignore` — ignore `src-tauri/tauri.*.local.conf.json` (per-developer Tauri overrides), plus Xcode-derived data, fastlane temp files, signing secrets

### Untouched in M1

- All React UI (no UI changes for M1 — the existing app just runs inside WKWebView)
- Chat server routes (`src-tauri/src/server.rs`) — M5 will add `/api/pair/apns` and `/api/chats/{id}/resume`
- Synapse / voice modules — no overlap with M1

---

## Task 1: Xcode free-signing setup (manual, ~10 min)

This task has **no code** and **no commit at the end** — it's a one-time Xcode configuration.

You need a free Apple ID, full Xcode installed (App Store → Xcode), and your iPhone connected via USB. No paid Apple Developer Program.

**Files:** None.

- [ ] **Step 1: Verify full Xcode is installed (not just Command Line Tools).**

```bash
xcode-select -p
```

Expected: a path ending in `/Xcode.app/Contents/Developer`. If you see `/Library/Developer/CommandLineTools`, install Xcode from the Mac App Store (~15 GB download), then run `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`.

- [ ] **Step 2: Sign into Xcode with your Apple ID.**

Open Xcode → **Settings…** (or `Cmd-,`) → **Accounts** tab → click `+` in the lower-left → **Apple ID** → enter your Apple ID and password.

After it loads, click your Apple ID on the left. You should see a "Personal Team" row appear under it with a 10-character team ID (looks like `ABCD1234EF`). Copy this Team ID — you'll need it for Task 2 if you haven't already filled in `tauri.conf.json`.

Close Settings.

- [ ] **Step 3: Capture the free Team ID and put it in your local Tauri override file.**

This repo is public and each contributor uses their own free Team ID, so the Team ID is **not** committed. Instead, it lives in a gitignored per-developer override file that Tauri 2 merges in at build time via the `--config` flag.

Create `src-tauri/tauri.ios.local.conf.json` (filename is matched by the `src-tauri/tauri.*.local.conf.json` rule in `.gitignore`):

```json
{
  "bundle": {
    "iOS": {
      "developmentTeam": "ABCD1234EF"
    }
  }
}
```

Replace `ABCD1234EF` with your real 10-character Team ID from Step 2.

Verify it parses:

```bash
python3 -m json.tool src-tauri/tauri.ios.local.conf.json > /dev/null && echo OK
```

Expected: `OK`.

Verify it's gitignored (should print nothing):

```bash
git status --porcelain src-tauri/tauri.ios.local.conf.json
```

No commit at this step — the file is intentionally untracked. The npm scripts `ios:dev` / `ios:build` (wired up in Task 2) pass `--config src-tauri/tauri.ios.local.conf.json` so the Team ID is merged into the build.

(Background: a free Personal Team ID is technically not a credential — it's embedded in every signed binary — but for a public repo we keep it out of the tree to avoid linking the repo to one developer's Apple ID identity and to make it obvious that contributors must use their own Team IDs. For a private repo or single-developer use, committing the Team ID directly into `tauri.conf.json` is fine.)

- [ ] **Step 4: Plug in your iPhone via USB and trust the Mac.**

Unlock the phone, tap "Trust" when prompted, confirm with your passcode. Verify:

```bash
xcrun xctrace list devices 2>&1 | grep -i iphone | head -3
```

Expected: your iPhone listed with a UDID.

- [ ] **Step 5: First-time device trust in Xcode (one-time).**

In Xcode → **Window → Devices and Simulators** → select your iPhone in the sidebar. Wait while Xcode prepares the device for development (~30s the first time — it copies a debug symbol package). The status indicator goes from yellow to green when ready. Close the window.

**Exit criteria for Task 1:**
- `xcode-select -p` returns the Xcode.app Developer path
- Your Apple ID is signed into Xcode with a "Personal Team" visible
- Your real 10-character Team ID is committed in `src-tauri/tauri.conf.json` (replacing the placeholder)
- Your iPhone shows up in Xcode's Devices window with a green status
- You're ready to do Tasks 6–9 (build + install) any time

**What you DO NOT have (intentional, per cost-deferral path):**
- $99 Apple Developer Program membership
- A registered `com.localmind.app` bundle identifier in the developer portal — Xcode auto-resolves to a wildcard provisioning profile for free signing
- Distribution provisioning profile
- An App Store Connect record
- The ability to use TestFlight, App Store, APNs push, or App Groups

These all gate on the paid account. When you reach M5 (push) or M6 (store), enroll then.

---

## Task 2: Add iOS bundle config to `tauri.conf.json`

**Files:**
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Read the current file to understand its structure.**

```bash
cat src-tauri/tauri.conf.json
```

Note the existing `bundle` section has `active`, `targets`, `icon`, `resources`, `macOS`. We're adding a peer `iOS` key.

- [ ] **Step 2: Add the iOS bundle section.** Insert a new `"iOS"` key under `"bundle"`, immediately after the existing `"macOS"` key.

Find the section that looks like:

```json
  "bundle": {
    "active": true,
    "targets": ["nsis", "app", "dmg", "deb", "appimage"],
    ...
    "macOS": { ... }
  }
```

Add this peer key:

```json
    "iOS": {
      "developmentTeam": "REPLACE_WITH_YOUR_TEAM_ID",
      "minimumSystemVersion": "15.0"
    }
```

The placeholder `REPLACE_WITH_YOUR_TEAM_ID` is intentional and **stays in the committed file** — the real Team ID lives in a per-developer gitignored override file (`src-tauri/tauri.ios.local.conf.json`, created in Task 1 Step 3) that Tauri merges in at build time via the `--config` flag wired up by the `ios:dev` / `ios:build` npm scripts in Task 2 Step 6.

- [ ] **Step 3: Validate the JSON parses.**

```bash
python3 -m json.tool src-tauri/tauri.conf.json > /dev/null && echo OK
```

Expected: `OK`. If you get a JSON error, fix the trailing comma or quote mismatch.

- [ ] **Step 4: Verify Tauri accepts the schema.**

```bash
cd src-tauri && cargo check --target aarch64-apple-ios 2>&1 | head -20
```

Expected: either successful compile-check, or errors that are NOT about `tauri.conf.json` schema (e.g. missing iOS toolchain is fine and expected — that's Task 6's problem; we just want to verify the conf file is well-formed from Tauri's perspective).

If the target isn't installed (`error: toolchain ... is not installed`), install it:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cd src-tauri && cargo check --target aarch64-apple-ios 2>&1 | head -20
```

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/tauri.conf.json
git commit -m "feat(ios): add iOS bundle config to tauri.conf.json"
```

- [ ] **Step 6: Wire `ios:dev` / `ios:build` npm scripts that merge in the per-developer Team ID.**

These scripts pass `--config src-tauri/tauri.ios.local.conf.json` to the underlying `tauri ios` CLI, so each developer's gitignored override file (from Task 1 Step 3) supplies the real Team ID at build time.

Edit `package.json` and add two entries to `scripts`, just under the existing `"tauri": "tauri"` line:

```json
    "ios:dev": "tauri ios dev --config src-tauri/tauri.ios.local.conf.json",
    "ios:build": "tauri ios build --config src-tauri/tauri.ios.local.conf.json",
```

Also add the gitignore rule that keeps every developer's override out of the tree. In the root `.gitignore`, under the existing "Environment / secrets" block, add:

```
# Per-developer Tauri config overrides (e.g. iOS free-signing Team ID)
src-tauri/tauri.*.local.conf.json
```

Verify the gitignore is working — your local override file from Task 1 Step 3 should not appear in `git status`:

```bash
git status --porcelain src-tauri/tauri.ios.local.conf.json
```

Expected: nothing. (If you see the file listed, the gitignore rule isn't matching — check spelling.)

Finally, write the per-developer onboarding doc (`docs/ios-developer-setup.md`) that walks future contributors through the Xcode install, Apple ID sign-in, Team ID extraction, local override creation, and device trust. This is the document a new contributor reads once when they first build for iOS. (Content: roughly the Task 1 + Task 7/8 manual steps, distilled into a 5-step checklist.)

Then commit:

```bash
git add package.json .gitignore docs/ios-developer-setup.md
git commit -m "feat(ios): per-developer Team ID via gitignored tauri.ios.local.conf.json"
```

---

## Task 3: Add iOS-specific `Info.plist` keys

**Context:** Tauri 2 mobile generates `Info.plist` inside `src-tauri/gen/apple/localmind_iOS/Info.plist`. The cleanest place to put project-managed additions is directly in that generated file — the `gen/apple/` tree is checked into git (`.gitignore` does not currently ignore it), so edits persist. If a future `tauri ios init` regenerates the file, this task will need to be re-applied; we mitigate this by also adding a guard comment.

**Files:**
- Modify: `src-tauri/gen/apple/localmind_iOS/Info.plist`

- [ ] **Step 1: Inspect the generated Info.plist.**

```bash
cat src-tauri/gen/apple/localmind_iOS/Info.plist
```

You'll see a standard plist with `CFBundleDisplayName`, `CFBundleIdentifier`, etc. We will add five new top-level dict entries.

- [ ] **Step 2: Add the privacy usage description for microphone.** Insert before the closing `</dict>`:

```xml
	<key>NSMicrophoneUsageDescription</key>
	<string>LocalMind uses your microphone to transcribe voice input locally on your device.</string>
```

(String matches the desktop `Info.macos.plist` for consistency.)

- [ ] **Step 3: Add the camera usage description.** Used by the QR scanner in M2; declaring it now means we won't forget.

```xml
	<key>NSCameraUsageDescription</key>
	<string>LocalMind uses your camera to scan pairing QR codes from your computer.</string>
```

- [ ] **Step 4: Add the local-network usage description.** Required by iOS 14+ before any LAN discovery / connection.

```xml
	<key>NSLocalNetworkUsageDescription</key>
	<string>LocalMind connects to your computer running LocalMind over the local network.</string>
```

- [ ] **Step 5: Declare the Bonjour service type.** M2 will use `_localmind._tcp`; declaring it now is harmless.

```xml
	<key>NSBonjourServices</key>
	<array>
		<string>_localmind._tcp</string>
	</array>
```

- [ ] **Step 6: Add the ATS local-networking exception.** Permits plain HTTP to RFC1918 / `.local` addresses. Without this, iOS blocks LAN HTTP.

```xml
	<key>NSAppTransportSecurity</key>
	<dict>
		<key>NSAllowsLocalNetworking</key>
		<true/>
	</dict>
```

- [ ] **Step 7: Validate the plist parses.**

```bash
plutil -lint src-tauri/gen/apple/localmind_iOS/Info.plist
```

Expected: `src-tauri/gen/apple/localmind_iOS/Info.plist: OK`. Any error → fix the XML.

- [ ] **Step 8: Verify all five keys are present.**

```bash
for key in NSMicrophoneUsageDescription NSCameraUsageDescription NSLocalNetworkUsageDescription NSBonjourServices NSAppTransportSecurity; do
  /usr/libexec/PlistBuddy -c "Print :$key" src-tauri/gen/apple/localmind_iOS/Info.plist > /dev/null 2>&1 \
    && echo "OK $key" || echo "MISSING $key"
done
```

Expected: five `OK` lines, no `MISSING`.

- [ ] **Step 9: Commit.**

```bash
git add src-tauri/gen/apple/localmind_iOS/Info.plist
git commit -m "feat(ios): add Info.plist privacy keys + local-networking ATS exception"
```

---

## Task 4: Create iOS capability allow-list

**Context:** Tauri 2 capability files define which commands the webview can invoke. The existing `capabilities/default.json` targets desktop windows. iOS uses a separate capability file so we can keep iOS-only commands (which arrive in M2+) cleanly separated.

**Files:**
- Create: `src-tauri/capabilities/ios.json`
- Modify: `src-tauri/capabilities/default.json` (scope to desktop platforms)

- [ ] **Step 1: Read the current default capability.**

```bash
cat src-tauri/capabilities/default.json
```

It currently has no `platforms` key, meaning it applies to all platforms. We'll restrict it to desktop and add a peer `ios.json` for iOS.

- [ ] **Step 2: Scope `default.json` to desktop platforms.** Add a `platforms` field at the top level:

Change the file from:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
```

to:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window (desktop)",
  "platforms": ["macOS", "windows", "linux"],
  "windows": ["main"],
  "permissions": [
```

The `permissions` array below stays unchanged.

- [ ] **Step 3: Create `src-tauri/capabilities/ios.json`.**

```json
{
  "$schema": "../gen/schemas/mobile-schema.json",
  "identifier": "ios",
  "description": "Capability for the main window (iOS)",
  "platforms": ["iOS"],
  "windows": ["main"],
  "permissions": [
    "core:default",
    "core:event:default",
    "core:window:default",
    "core:app:default",
    "dialog:default",
    "store:default"
  ]
}
```

Notes on the iOS permission set:
- `opener:default` (URL/file opener) — omitted for now; iOS uses `UIApplication.shared.open` differently. Re-evaluate in M3.
- `fs:default` — omitted for now; iOS file system is sandboxed and the chat path doesn't need raw FS access. Re-evaluate if RAG-on-mobile ever lands.
- `shell:default` — omitted; iOS doesn't run subprocesses.

- [ ] **Step 4: Validate both files parse.**

```bash
python3 -m json.tool src-tauri/capabilities/default.json > /dev/null && \
python3 -m json.tool src-tauri/capabilities/ios.json > /dev/null && \
echo OK
```

Expected: `OK`.

- [ ] **Step 5: Verify Tauri accepts the new capability structure.**

```bash
cd src-tauri && cargo check 2>&1 | grep -iE "capabilit|permission|error" | head -10
```

Expected: no errors mentioning capabilities or permissions. Pre-existing warnings are fine.

- [ ] **Step 6: Commit.**

```bash
git add src-tauri/capabilities/default.json src-tauri/capabilities/ios.json
git commit -m "feat(ios): add iOS capability file; scope default to desktop"
```

---

## Task 5: Scaffold the iOS-only Rust module

**Context:** M2 onwards will add Tauri commands like `ios_scan_qr`, `keychain_store`, etc. We pre-build the empty module gate now so M2 starts on a known-good baseline. The module is `#[cfg(target_os = "ios")]`-gated so it's a no-op for desktop builds.

**Files:**
- Create: `src-tauri/src/ios/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing test.** Create the iOS module with one trivial test that exercises the gate. The test is `#[cfg(target_os = "ios")]` so it only runs in iOS builds, but the module itself compiles unconditionally (the cfg gate is at the *use site* in `lib.rs`).

Create `src-tauri/src/ios/mod.rs`:

```rust
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
```

- [ ] **Step 2: Run the test on desktop and confirm it does NOT run (module is iOS-only).**

```bash
cd src-tauri && cargo test --lib ios 2>&1 | tail -5
```

Expected: `0 passed; 0 failed; 0 ignored; 0 measured; 44 filtered out`. The `ios::tests::version_is_v1` test is filtered out because the module isn't compiled on desktop (we haven't wired it in `lib.rs` yet — that's the next step).

- [ ] **Step 3: Wire the module into `lib.rs`.** Open `src-tauri/src/lib.rs` and find the module declarations near the top of the file. Add:

```rust
#[cfg(target_os = "ios")]
mod ios;
```

Place it in **alphabetical order** with the other `mod` declarations — between `mod host_proxy;` and `mod llama;`. rustfmt enforces strict alphabetical ordering for module declarations even with `#[cfg(...)]` attributes, so don't append it at the end of the block (CI will fail `cargo fmt --all -- --check`).

Then find the `invoke_handler` call (likely in the `run()` function in `lib.rs`) — it'll look like:

```rust
.invoke_handler(tauri::generate_handler![
    health,
    ...
    voice_save_recording,
    ...
])
```

Add the iOS command (cfg-gated):

```rust
.invoke_handler(tauri::generate_handler![
    health,
    ...
    voice_save_recording,
    #[cfg(target_os = "ios")]
    ios::ios_bridge_version,
    ...
])
```

- [ ] **Step 4: Verify desktop build still compiles (no iOS regression).**

```bash
cd src-tauri && cargo build 2>&1 | tail -5
```

Expected: `Finished \`dev\` profile`. No errors.

- [ ] **Step 5: Verify iOS target compiles.**

```bash
cd src-tauri && cargo check --target aarch64-apple-ios 2>&1 | tail -10
```

Expected: `Finished \`dev\` profile` for the iOS target, with the `ios` module now in scope. If you see `unresolved import \`crate::ios\``, the `lib.rs` wiring in Step 3 is wrong.

- [ ] **Step 6: Run desktop tests to confirm no regression.**

```bash
cd src-tauri && cargo test --lib 2>&1 | tail -3
```

Expected: `test result: ok. 44 passed; 0 failed; 0 ignored`. (The count was 44 after the voice parser fix; if you're working against a later main, adjust expected count to whatever the baseline is.)

- [ ] **Step 7: Commit.**

```bash
git add src-tauri/src/ios/mod.rs src-tauri/src/lib.rs
git commit -m "feat(ios): scaffold cfg-gated iOS Rust module + version probe"
```

---

## Task 6: Validate the Tauri iOS scaffolding

**Context:** `src-tauri/gen/apple/` was generated ~2 weeks ago via `tauri ios init`. Confirm it still works against today's Tauri version, or re-init if Tauri has bumped.

**Files:**
- Modify: `src-tauri/gen/apple/*` (only if re-init is needed)

- [ ] **Step 1: Check Tauri CLI version.**

```bash
npm exec tauri -- --version
```

Note the version (e.g. `2.x.y`).

- [ ] **Step 2: Check the version Tauri was when `gen/apple/` was generated.**

```bash
git log --oneline -1 src-tauri/gen/apple/project.yml
```

If the generation commit is older than the most recent Tauri CLI bump (`git log --oneline -- package-lock.json | head -5` and look for `@tauri-apps/cli` updates), proceed to Step 3 (re-init). Otherwise skip to Step 5.

- [ ] **Step 3: Re-initialize iOS scaffolding (only if Step 2 says it's stale).**

```bash
# Back up the current gen/apple dir
mv src-tauri/gen/apple src-tauri/gen/apple.bak
npm run tauri ios init
```

Expected: scaffolding regenerates without prompting. If it prompts for team ID, paste the one from Task 1 Step 3.

- [ ] **Step 4: Diff against the backup to see what changed.**

```bash
diff -ruN src-tauri/gen/apple.bak src-tauri/gen/apple | head -100
```

Re-apply the Info.plist changes from Task 3 if the diff shows the new file overwrote them. Then:

```bash
rm -rf src-tauri/gen/apple.bak
```

- [ ] **Step 5: Open the Xcode project to verify it loads cleanly.**

```bash
open src-tauri/gen/apple/localmind.xcodeproj
```

Wait for Xcode to finish indexing (status bar in the top center). Verify:
- The `localmind_iOS` scheme appears in the scheme picker (top-left, next to the play button).
- Under Targets → `localmind_iOS` → **Signing & Capabilities**:
  - **Automatically manage signing** — checked.
  - **Team** — pick your "Personal Team" (the one tied to your free Apple ID; same Team ID you put in `tauri.conf.json`).
  - **Bundle Identifier** — should read `com.localmind.app`.
  - **Provisioning Profile** — should show "Xcode Managed Profile" (no error). If you see a red banner ("Failed to register bundle identifier" or "No profiles found"), that's expected on the first build attempt — proceed to Task 7 and let Xcode auto-resolve. If it doesn't resolve after a build attempt, try changing the Bundle Identifier to something unique (e.g. `com.localmind.app.<your-initials>`) — free-tier wildcard provisioning sometimes conflicts with previously-used bundle IDs.

Close Xcode when done (you don't need to build from inside Xcode — Tauri's CLI drives it).

- [ ] **Step 6: Commit any changes (re-init may have updated files).**

```bash
git status src-tauri/gen/apple/
# If anything changed:
git add src-tauri/gen/apple/
git commit -m "chore(ios): refresh tauri ios scaffolding"
# Otherwise:
echo "no changes — scaffolding is up to date"
```

---

## Task 7: First local dev build (iOS Simulator)

**Context:** Run the app in the iOS Simulator before attempting a device build. This catches all the schema / config / linking issues without involving signing.

**Files:** None modified — pure verification task.

- [ ] **Step 1: Verify Xcode command-line tools are installed.**

```bash
xcode-select -p
```

Expected: a path like `/Applications/Xcode.app/Contents/Developer`. If it returns `/Library/Developer/CommandLineTools`, run `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer` first.

- [ ] **Step 2: List available simulators.**

```bash
xcrun simctl list devices available | head -20
```

Pick an iOS 15+ simulator — e.g. `iPhone 15` running iOS 17. Note its UDID (the bracketed long string at the end of its line).

- [ ] **Step 3: Boot the simulator.**

```bash
xcrun simctl boot <UDID>
open -a Simulator
```

- [ ] **Step 4: Run `tauri ios dev` against the simulator.**

```bash
npm run tauri ios dev -- --open
```

Tauri builds the iOS app (first build is slow — 5–10 min on M1 Pro), installs it into the simulator, and launches it. The `--open` flag also opens Xcode (useful for the logs panel).

Expected (in the simulator window): the existing LocalMind Connect screen — the same one that appears on the desktop's first launch — rendered inside WKWebView.

- [ ] **Step 5: Verify the React UI is functional.** Inside the simulator:
- The app icon shows on the home screen (use the home button to background and back — it should say `LocalMind`).
- The Connect screen shows the PIN entry field, QR scan area, etc.
- DON'T attempt to actually pair yet — the simulator has no LAN connection to your desktop server and M2 hasn't built the discovery flow yet. We're only validating that the React bundle ships and renders.

- [ ] **Step 6: Verify the iOS console for errors.** In Xcode (the `--open` flag should have opened it), check the Debug Area (View → Debug Area → Show Debug Area). Look for any RED error lines.

Common harmless warnings you can ignore: `WKWebView did fail navigation`, `Could not load NSBundle` for missing fonts, `App Transport Security has blocked` (for non-local hosts the app is innocently trying to reach — fine).

Real errors to fix: anything mentioning `Info.plist`, `entitlements`, `capability`, `Tauri command not found`, `bridge`, `cargo`. If you see one, stop and debug before continuing.

- [ ] **Step 7: Stop the dev build.** Hit Ctrl-C in the terminal running `tauri ios dev`. The simulator app will close. This task has no commit — it's a verification gate.

**Exit criteria for Task 7:** LocalMind launches in the iOS Simulator and shows the Connect screen with no console errors.

---

## Task 8: First device build via free signing

**Context:** Install LocalMind on your physical iPhone using free Xcode signing. No `.ipa` archive needed — we use Tauri's dev workflow which auto-installs to the connected device. The build is debug-mode (not release-mode), which is fine for personal-device testing.

**Files:** None modified.

- [ ] **Step 1: Verify Task 1 prerequisites are done.** Your iPhone is plugged in, trusted, and visible:

```bash
xcrun xctrace list devices 2>&1 | grep -i iphone | head -3
```

Expected: your iPhone listed with a UDID.

- [ ] **Step 2: Verify Xcode signing config one last time.**

```bash
open src-tauri/gen/apple/localmind.xcodeproj
```

In Xcode → Targets → `localmind_iOS` → Signing & Capabilities:
- "Automatically manage signing" — checked.
- Team — your Personal Team.
- Bundle Identifier — `com.localmind.app` (if it errors, change to `com.localmind.app.<your-initials>` to dodge bundle-ID collisions on the free wildcard provisioning).
- Provisioning Profile — should resolve to "Xcode Managed Profile". If it shows a red banner, click "Try Again" and wait for Xcode to talk to Apple.

Close Xcode.

- [ ] **Step 3: Run the dev build pointed at your phone.**

```bash
npm run tauri ios dev
```

When prompted "Detected connected device. Launch on it?" — answer **y**. (Or pass `--host` and a specific device ID; see `tauri ios dev --help`.)

The first build takes 10–20 min on M1 Pro (full Rust debug build for iOS + Swift compile + signing + install). Subsequent builds are 30s–2 min thanks to Cargo's incremental compilation.

When the build finishes, the app launches automatically on your iPhone.

- [ ] **Step 4: Handle the "Untrusted Developer" prompt (one-time per Apple ID).**

If the app fails to launch with "Untrusted Developer", that's normal for the first install:
- iPhone: Settings → General → VPN & Device Management → tap your Apple ID under DEVELOPER APP → Trust "Apple Development: yourname@yourdomain" → Trust.
- Re-launch the app from the home screen.

- [ ] **Step 5: Verify the app launches and the Connect screen renders.**

Look for the LocalMind home screen icon. Tap to open. The Connect screen should appear (PIN entry, QR scan area, etc.) — same as the existing PWA experience because we're hosting the same React bundle inside WKWebView.

DON'T attempt to actually pair yet — M2 hasn't built the discovery flow. We're only validating that the React bundle ships and renders on the device.

- [ ] **Step 6: Check the iOS console for errors.**

The terminal running `npm run tauri ios dev` streams device logs. Look for any RED error lines.

Common harmless warnings: `WKWebView did fail navigation` (for non-existent endpoints during initial load), `App Transport Security has blocked` (for any non-local URLs the app tries to reach in code paths we haven't excluded yet).

Real errors to fix: anything mentioning `Info.plist`, `entitlements`, `capability`, `Tauri command not found`, `bridge`, or panics from Rust code.

- [ ] **Step 7: Stop the dev session.** Hit Ctrl-C in the terminal. The app stays installed on your phone (it just stops live-reloading). You can re-launch it from the home screen any time within 7 days; after that, re-run `npm run tauri ios dev` to re-sign and re-install.

- [ ] **Step 8: Commit any incidental changes from Xcode opening the project.**

```bash
git status src-tauri/gen/apple/
# If anything changed:
git add src-tauri/gen/apple/
git commit -m "chore(ios): xcode auto-managed signing settings"
# Otherwise:
echo "no changes — proceed"
```

**Exit criteria for Task 8:** LocalMind launches on your physical iPhone via free Xcode signing, with no install or runtime crash, displaying the Connect screen.

**This is the M1 deliverable for the free-signing path.** Task 9 (TestFlight) is deferred to M5/M6 when you pay the $99. Task 10 (CI) is similarly deferred.

---

## Task 9 (deferred): First TestFlight upload

**Status: DEFERRED until you pay the $99 Apple Developer Program fee** (likely at M5 or M6).

TestFlight is the App Store's beta distribution channel. It requires a paid Apple Developer Program membership, a Distribution provisioning profile, and an App Store Connect record — none of which the free-signing path provides.

When you do enroll, this task becomes:
1. Generate an App Store Connect API key.
2. Rebuild with a Distribution signing identity (`npm run tauri ios build -- --target aarch64-apple-ios`).
3. Upload the `.ipa` via `xcrun altool --upload-app`.
4. Wait for processing in App Store Connect.
5. Install via TestFlight on your phone.

Full steps are preserved in version control history of this plan (pre-deferral revision). Don't worry about them now — focus on getting M2–M4 working with free signing first.

**M1 ships at the end of Task 8** under the free-signing path. Tag the completion when you finish Task 8:

```bash
git tag -a m1-ios-bootstrap-complete -m "M1 ships: first free-signed iOS build of LocalMind installed on personal device"
```

**Exit criteria for M1 (free-signing path):** LocalMind is installed on your physical iPhone via free Xcode signing and displays the Connect screen (Task 8 exit criteria).

---

## Task 10 (deferred): GitHub Actions CI for TestFlight uploads

**Status: DEFERRED.** CI uploads to TestFlight require a paid Distribution signing certificate and TestFlight access — both gated by the $99 enrollment. Revisit when you pay.

Free-signing builds cannot be meaningfully automated in CI either: a free-tier provisioning profile is tied to a specific machine's Keychain and re-signed every 7 days. There's no way to ship a free-signed `.ipa` from CI to your phone.

For now, your "CI" is `npm run tauri ios dev` on your laptop, run weekly when the previous build expires. That's the cost of deferring the $99.

When you do enroll, the original CI workflow (preserved below for reference) becomes viable.

**Files:**
- Create: `.github/workflows/ios.yml`

- [ ] **Step 1: Prepare repository secrets.**

In GitHub: Settings → Secrets and variables → Actions → New repository secret. Add:

- `APPLE_TEAM_ID` — your 10-char team ID
- `APPLE_API_KEY_ID` — the 10-char key ID from Task 9 Step 2
- `APPLE_API_ISSUER_ID` — the issuer UUID from Task 9 Step 2
- `APPLE_API_PRIVATE_KEY` — paste the entire contents of the `.p8` file (begins with `-----BEGIN PRIVATE KEY-----`)
- `APPLE_CERTIFICATE_P12` — your Apple Distribution certificate exported as `.p12`, then base64-encoded: `base64 -i certificate.p12 -o certificate.p12.b64; pbcopy < certificate.p12.b64`
- `APPLE_CERTIFICATE_PASSWORD` — the password you used when exporting the `.p12`
- `APPLE_PROVISIONING_PROFILE` — your Distribution `.mobileprovision`, base64-encoded similarly

- [ ] **Step 2: Create the workflow file.**

`.github/workflows/ios.yml`:

```yaml
name: iOS Build & TestFlight

on:
  push:
    branches: [main]
    paths-ignore:
      - 'docs/**'
      - '**.md'
  workflow_dispatch:

jobs:
  build:
    runs-on: macos-14
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4

      - name: Select Xcode
        run: sudo xcode-select -s /Applications/Xcode_16.app

      - name: Set up Node
        uses: actions/setup-node@v4
        with:
          node-version: '24'
          cache: 'npm'

      - name: Set up Rust + iOS targets
        run: |
          rustup target add aarch64-apple-ios
          echo "$HOME/.cargo/bin" >> $GITHUB_PATH

      - name: Install dependencies
        run: npm ci

      - name: Import signing certificate
        env:
          CERT_P12: ${{ secrets.APPLE_CERTIFICATE_P12 }}
          CERT_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
        run: |
          echo "$CERT_P12" | base64 -d > /tmp/cert.p12
          security create-keychain -p "" build.keychain
          security set-keychain-settings -lut 21600 build.keychain
          security unlock-keychain -p "" build.keychain
          security import /tmp/cert.p12 -k build.keychain -P "$CERT_PASSWORD" -T /usr/bin/codesign
          security set-key-partition-list -S apple-tool:,apple: -s -k "" build.keychain
          security list-keychain -d user -s build.keychain login.keychain
          rm /tmp/cert.p12

      - name: Install provisioning profile
        env:
          PROFILE: ${{ secrets.APPLE_PROVISIONING_PROFILE }}
        run: |
          mkdir -p ~/Library/MobileDevice/Provisioning\ Profiles
          echo "$PROFILE" | base64 -d > ~/Library/MobileDevice/Provisioning\ Profiles/LocalMindDistribution.mobileprovision

      - name: Build iOS release
        env:
          APPLE_DEVELOPMENT_TEAM: ${{ secrets.APPLE_TEAM_ID }}
        run: npm run tauri ios build -- --target aarch64-apple-ios

      - name: Upload to TestFlight
        env:
          API_KEY_ID: ${{ secrets.APPLE_API_KEY_ID }}
          API_ISSUER_ID: ${{ secrets.APPLE_API_ISSUER_ID }}
          API_PRIVATE_KEY: ${{ secrets.APPLE_API_PRIVATE_KEY }}
        run: |
          mkdir -p ~/.appstoreconnect/private_keys
          echo "$API_PRIVATE_KEY" > ~/.appstoreconnect/private_keys/AuthKey_${API_KEY_ID}.p8
          IPA=$(find src-tauri/gen/apple/build -name "*.ipa" -mtime -1 | head -1)
          xcrun altool --upload-app --type ios --file "$IPA" \
            --apiKey "$API_KEY_ID" --apiIssuer "$API_ISSUER_ID"
```

- [ ] **Step 3: Validate the workflow YAML parses.**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ios.yml'))" && echo OK
```

Expected: `OK`.

- [ ] **Step 4: Commit and push to trigger the first CI run.**

```bash
git add .github/workflows/ios.yml
git commit -m "ci(ios): build + upload to TestFlight on push to main"
git push
```

- [ ] **Step 5: Watch the run in GitHub Actions.** First run may surface issues:
- Xcode version mismatch (`macos-14` runner may have Xcode 15.x; bump to `macos-15` if 16 isn't there yet).
- Provisioning profile UUID mismatch (regenerate from developer portal).
- API key permissions insufficient (re-create with Developer or App Manager role).

Iterate as needed. Each fix is a small commit + push to re-trigger.

**Exit criteria for Task 10:** A push to `main` produces a TestFlight build with no manual intervention.

---

## Spec coverage check

Mapping spec §8 M1 exit criteria to plan tasks (free-signing path):

- "Apple Developer setup" → Task 1 (now free Xcode signing, not paid enrollment) ✓
- "`tauri ios init` already done, fix capabilities + Info.plist additions" → Tasks 3, 4 ✓
- "first build that launches WKWebView and shows existing Connect screen" → Task 8 ✓ (TestFlight removed from criteria; install path is direct device install via free signing)
- "Exit criteria: you can install on your phone and see the React app" → Task 8 Step 5 ✓

Mapping spec §5 ("Bundle layout", "Native modules", "Info.plist additions") subset that M1 must cover:

- Main `LocalMind.app` bundle ID `com.localmind.app` (or `.<your-initials>` if free wildcard collides) → Task 1, Task 8 ✓
- iOS minimum 15.0 → Task 2 ✓
- All five Info.plist privacy/ATS keys → Task 3 ✓
- iOS-only Rust module skeleton → Task 5 ✓
- `npm run tauri ios dev` device install pipeline → Task 8 ✓

Deferred to later milestones (correctly out of M1 scope):

- Native Swift modules (QR, mDNS, Keychain, etc.) → M2/M3/M4/M5
- Share extension target → M3 *(also needs paid account — App Groups; M3 plan must either drop the share extension or include the $99 enrollment)*
- `apns_pusher` Rust crate → M5 *(needs paid account)*
- Server-side endpoints (`/api/pair/apns`, `/api/chats/{id}/resume`) → M4/M5
- Mobile UI breakpoints → M3
- Privacy manifest + App Store metadata → M6 *(needs paid account)*

Deferred specifically because of the cost-deferral path:

- TestFlight upload pipeline → Task 9 (deferred until paid enrollment)
- GitHub Actions CI for builds → Task 10 (deferred until paid enrollment)

No spec requirement is silently dropped — the spec assumed paid account, this plan honors the cost-deferral decision while preserving every spec-listed feature for future implementation.

---

## What ships after this plan (free-signing path)

After Task 8 completes:

- A locally-signed LocalMind iOS app is installed on your iPhone via Xcode "Personal Team" signing.
- It hosts the existing React UI inside WKWebView and shows the Connect screen.
- The app must be reinstalled every 7 days (free signing limitation) — `npm run tauri ios dev` handles this.
- It does NOT yet pair with the desktop (no QR scanner, no mDNS browser — that's M2).
- It does NOT yet have the native polish (no FaceID, no share extension, no mobile breakpoints — that's M3).
- It does NOT push notifications, App Store presence, or share extension (those need M5/M6 + paid account).
- The build pipeline is validated end-to-end, so M2 starts on a known-good baseline.

After Task 10 (if completed):

- Every push to `main` produces a TestFlight build automatically.

The next plan to write: `docs/superpowers/plans/<date>-tauri-mobile-ios-m2-pairing.md`, written when this plan completes so it reflects what we actually learned during M1.

---

## Discovered during M1 — track for M2+

Items surfaced while executing this plan that aren't M1 scope but must not be lost when the next plans are written.

- **Back navigation missing on iOS (M2/M3).** The React app uses Zustand state-based view switching (no React Router / no browser history), and iOS has no hardware back button. Result: once a user navigates into Settings, model detail, or any sub-view, there is no way out except restarting the app. M2's pairing flow (multi-step: QR scan → PIN → confirm) will feel this acutely. Two options to evaluate when M2 lands:
  1. Add explicit per-view back buttons in the React UI (mobile-only via breakpoint), feeding a "previous view" stack the Zustand store maintains. Lowest-risk; works the same on iOS and Android.
  2. Migrate the React app to URL-based routing (React Router or TanStack Router), enable `WKWebView.allowsBackForwardNavigationGestures = true` in the iOS shell, and let iOS's edge-swipe gesture handle back. More invasive; touches desktop too.
  Recommend (1) for M2 (pairing-flow-only), defer (2) to M3 if/when mobile polish demands it.
- **Stale keychain cert trap for free-signing.** During M1, my keychain had an `Apple Development: ... (34SJ7XNNQJ)` cert from a previously-revoked Personal Team. The current Personal Team Xcode actually had account-level access to was `D97QT44UZB`, evidenced by the auto-created provisioning profile in `~/Library/Developer/Xcode/UserData/Provisioning Profiles/*.mobileprovision`. Reading the Team ID off the keychain cert (`security find-identity`) was misleading; the **provisioning profile's `TeamIdentifier`** is the authoritative source. The `docs/ios-developer-setup.md` instructions should reflect this (and do, after the M1 fix). Worth a project-wide note that any future iOS-signing debugging starts with `security cms -D -i <profile>.mobileprovision | grep -A1 TeamIdentifier`, not with the keychain.
- **iOS dev-mode LAN permission is a UX gotcha (M3 polish).** First-launch of a dev build hits `http://<mac-LAN-IP>:1420/` for the React bundle and triggers iOS's Local Network permission alert. If the user dismisses it accidentally, the app appears broken with a wall-of-text error rather than a recoverable UI. Production builds don't have this (bundle is embedded), but M3 should ensure the dev-mode error screen is friendlier and gives a "open Settings" deep link.
- **`tauri ios dev` tears down Vite on launch failure.** If the install/launch step fails (e.g. Developer Mode off, Untrusted Developer), the Vite dev server is killed too. The app on the device is then stranded — you can launch it from the home screen but it can't fetch the bundle. Workaround: re-run `npm run ios:dev` to bring everything back up together. Not a blocker but worth documenting in `docs/ios-developer-setup.md` as a troubleshooting note.
