# Tauri Mobile iOS — Milestone 1: Bootstrap & Build

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Get a signed iOS build of LocalMind onto a real iPhone via TestFlight, hosting the existing React app inside WKWebView with no functional regressions on desktop.

**Architecture:** Use the Tauri 2 mobile scaffolding already in `src-tauri/gen/apple/`. Add an `iOS` section to `tauri.conf.json`, layer in iOS-specific `Info.plist` entries (mic, camera, local network, ATS), create an iOS-only capability allow-list, scaffold a cfg-gated Rust module for future iOS-only commands, and validate the build chain end-to-end. No new product features in M1 — this milestone exists to de-risk the iOS pipeline before M2 (pairing & discovery) starts.

**Tech Stack:** Tauri 2.x mobile (iOS target), Xcode 16+, Swift 5.9+, Rust (existing `src-tauri/` crate, `target_os = "ios"` cfg gates), React 19 (existing, unchanged), Apple Developer Program (paid).

**Spec:** [`docs/superpowers/specs/2026-05-11-tauri-mobile-ios-parity-design.md`](../specs/2026-05-11-tauri-mobile-ios-parity-design.md), §5 ("iOS Surfaces & Build Pipeline") and §8 ("Phased Milestones") M1.

**M2–M6 plans:** Not in scope here. Each subsequent milestone gets its own plan written when M1 ships, so the plan reflects the actual rough edges discovered during M1.

---

## File Structure

### Created

- `src-tauri/Info.ios.plist` — iOS-only Info.plist key additions, merged by Tauri at build time
- `src-tauri/capabilities/ios.json` — iOS-only Tauri capability allow-list
- `src-tauri/src/ios/mod.rs` — iOS-only Rust module gate (empty in M1; populated by M2+)
- `docs/ios-developer-setup.md` — written-down checklist for the manual Apple Developer Program steps (so anyone joining the project can do this once)
- `.github/workflows/ios.yml` — GitHub Actions workflow for building and uploading TestFlight builds on push to `main` *(M1 stretch — falls through to manual upload if not done in M1)*

### Modified

- `src-tauri/tauri.conf.json` — add `bundle.iOS` section (development team, min iOS version, simulator vs device build targets)
- `src-tauri/src/lib.rs` — wire `#[cfg(target_os = "ios")] mod ios;`
- `package.json` — convenience scripts (`ios:dev`, `ios:build`) wrapping the underlying `tauri ios` commands
- `README.md` — extend the "Going fully native (Tauri Mobile)" section with concrete commands and prerequisites
- `.gitignore` — Xcode-derived data, fastlane temp files, signing secrets

### Untouched in M1

- All React UI (no UI changes for M1 — the existing app just runs inside WKWebView)
- Chat server routes (`src-tauri/src/server.rs`) — M5 will add `/api/pair/apns` and `/api/chats/{id}/resume`
- Synapse / voice modules — no overlap with M1

---

## Task 1: Apple Developer Program Setup (manual, one-time)

This task has **no code** and **no commit at the end** — it's external-system setup. But it is a hard prerequisite for every subsequent task in this plan, so it ships as Task 1.

**Files:**
- Create: `docs/ios-developer-setup.md` (the only on-repo artifact for this task)

- [ ] **Step 1: Verify you do not already have an Apple Developer account.** Visit https://developer.apple.com/account/. If you're enrolled, skip ahead to Step 4.

- [ ] **Step 2: Enroll in the Apple Developer Program.** $99/year. Use your Apple ID (the one tied to your phone for TestFlight installs). The enrollment review takes 24–48 hours typically. **You cannot proceed past Task 4 until this completes.**

- [ ] **Step 3: Once enrolled, capture your Team ID.** Visit https://developer.apple.com/account/#/membership. Copy the 10-character Team ID (looks like `ABCD1234EF`). You'll paste it into `tauri.conf.json` in Task 2.

- [ ] **Step 4: Register the app's bundle identifier.** Visit https://developer.apple.com/account/resources/identifiers/list. Click `+`, choose **App IDs**, type **App**, continue.
   - Description: `LocalMind`
   - Bundle ID: **Explicit** → `com.localmind.app` (must match `tauri.conf.json` `identifier`)
   - Capabilities to enable: leave defaults for now. M5 will add **Push Notifications** and **App Groups** when needed.
   - Click Continue → Register.

- [ ] **Step 5: Create a Development Provisioning Profile.** Visit https://developer.apple.com/account/resources/profiles/list. Click `+`, type **iOS App Development**, continue.
   - App ID: `com.localmind.app`
   - Certificates: select your Apple development certificate (Xcode will have created one in your Keychain when you first opened Xcode; if not, Xcode → Settings → Accounts → Add Apple ID → Manage Certificates → `+` → Apple Development).
   - Devices: add your iPhone's UDID (find via Finder → connect phone → click device name once in the sidebar → the long string above Capacity).
   - Name: `LocalMind Dev`
   - Generate and download. Double-click the `.mobileprovision` to install it.

- [ ] **Step 6: Create a Distribution Provisioning Profile.** Same flow, but choose **App Store** under Distribution.
   - App ID: `com.localmind.app`
   - Certificates: select your Apple Distribution certificate (create one via Xcode if missing).
   - Name: `LocalMind Distribution`
   - Generate, download, install.

- [ ] **Step 7: Create the App Store Connect record.** Visit https://appstoreconnect.apple.com/apps. Click `+` → New App.
   - Platform: iOS
   - Name: `LocalMind`
   - Primary Language: English (US)
   - Bundle ID: `com.localmind.app`
   - SKU: `localmind-ios` (any unique string)
   - User Access: Full Access
   - Submit. This record is what TestFlight uploads attach to.

- [ ] **Step 8: Write `docs/ios-developer-setup.md`.** Document Steps 1–7 in your own words so this is reproducible for future contributors. Save the file, then:

```bash
git add docs/ios-developer-setup.md
git commit -m "docs(ios): add Apple Developer Program setup checklist"
```

**Exit criteria for Task 1:**
- You have a 10-character Team ID written down.
- A `com.localmind.app` bundle identifier is registered in the developer portal.
- Two provisioning profiles (`LocalMind Dev` + `LocalMind Distribution`) are installed on your machine.
- A `LocalMind` app record exists in App Store Connect.

---

## Task 2: Add iOS bundle config to `tauri.conf.json`

**Files:**
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Read the current file to understand its structure.**

```bash
cat src-tauri/tauri.conf.json
```

Note the existing `bundle` section has `active`, `targets`, `icon`, `resources`, `macOS`. We're adding a peer `iOS` key.

- [ ] **Step 2: Add the iOS bundle section.** Insert a new `"iOS"` key under `"bundle"`, immediately after the existing `"macOS"` key. The Team ID is the value you captured in Task 1, Step 3.

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

Replace `REPLACE_WITH_YOUR_TEAM_ID` with your actual Team ID from Task 1 Step 3.

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

Place it alongside the other `mod` declarations (e.g. near `mod voice;`, `mod voice_audio;`).

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
- Under Targets → `localmind_iOS` → **Signing & Capabilities**: the Team is set to your Apple Developer team (auto-populated from `tauri.conf.json`). If empty, click the Team dropdown and pick your team.

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

## Task 8: First release IPA build (device, signed)

**Context:** Build a signed `.ipa` that can be installed on a physical device via TestFlight. This validates the full signing pipeline before we hand off to TestFlight in Task 9.

**Files:** None modified.

- [ ] **Step 1: Plug in your iPhone via USB and trust it.** Unlock the phone, trust the Mac when prompted. Verify it appears:

```bash
xcrun xctrace list devices 2>&1 | grep -i iphone | head -3
```

Expected: your iPhone listed with a UDID. If not, check the cable / unlock state.

- [ ] **Step 2: Open Xcode and verify signing.**

```bash
open src-tauri/gen/apple/localmind.xcodeproj
```

In Xcode → Targets → `localmind_iOS` → Signing & Capabilities:
- "Automatically manage signing" — check it.
- Team — set to your Apple Developer team.
- Bundle Identifier — confirm `com.localmind.app`.
- Provisioning Profile — should auto-resolve to `LocalMind Dev` for Debug and `LocalMind Distribution` for Release. If it shows red errors, click "Try Again" or set them manually (Build Settings → Signing → "Provisioning Profile" rows).

Close Xcode.

- [ ] **Step 3: Build the release IPA.**

```bash
npm run tauri ios build -- --target aarch64-apple-ios
```

This takes 10–20 min on M1 Pro (Rust release build + Xcode archive + IPA pack). Output location at end: `src-tauri/gen/apple/build/arm64/LocalMind.ipa` (path may vary; Tauri prints it).

- [ ] **Step 4: Confirm the IPA exists and is signed.**

```bash
find src-tauri/gen/apple/build -name "*.ipa" -mtime -1
```

Expected: a path to the just-built `.ipa`. Then verify it's signed:

```bash
unzip -p $(find src-tauri/gen/apple/build -name "*.ipa" -mtime -1 | head -1) Payload/LocalMind.app/_CodeSignature/CodeResources | head -5
```

Expected: XML content (the signature manifest). If `unzip` complains "no such file", the app inside the IPA is unsigned and signing failed in Step 3 — re-check Xcode signing settings.

- [ ] **Step 5: Verify the embedded Info.plist contains the keys from Task 3.**

```bash
IPA=$(find src-tauri/gen/apple/build -name "*.ipa" -mtime -1 | head -1)
unzip -p "$IPA" Payload/LocalMind.app/Info.plist | plutil -convert xml1 -o - - | grep -E "NSMicrophone|NSCamera|NSLocalNetwork|NSBonjourServices|NSAppTransportSecurity"
```

Expected: at least one line for each of the five keys you added. If any are missing, Tauri's build didn't pick them up from `gen/apple/.../Info.plist` — re-check Task 3 (file path correct?) and rebuild.

- [ ] **Step 6: Install the IPA on your phone via Xcode Devices window.**

In Xcode: Window → Devices and Simulators → select your iPhone → drag the `.ipa` from Finder onto the "Installed Apps" list. Wait for install to complete (~30s).

- [ ] **Step 7: Launch LocalMind on your phone.** First launch will:
- Show "Untrusted Developer" the first time. Fix: Settings → General → VPN & Device Management → tap your Apple ID under DEVELOPER APP → Trust.
- Re-launch. Connect screen should appear.

- [ ] **Step 8: Commit any incidental changes from Xcode opening the project.**

```bash
git status src-tauri/gen/apple/
# If anything changed:
git add src-tauri/gen/apple/
git commit -m "chore(ios): xcode signing settings auto-update"
```

**Exit criteria for Task 8:** LocalMind launches on your physical iPhone with no install or runtime crash, displaying the Connect screen.

---

## Task 9: First TestFlight upload

**Context:** TestFlight is the App Store's beta-distribution channel and the staging area for App Store releases. Uploading once now validates the App Store Connect integration before M6's actual store submission.

**Files:** None modified.

- [ ] **Step 1: Verify your release IPA is up to date.** Use the one built in Task 8, or rebuild:

```bash
ls -lt src-tauri/gen/apple/build/*/LocalMind.ipa 2>/dev/null | head -1
```

If older than your latest commit, rebuild with `npm run tauri ios build -- --target aarch64-apple-ios`.

- [ ] **Step 2: Generate an App Store Connect API key (one-time).**

Visit https://appstoreconnect.apple.com/access/integrations/api → Generate API Key. Name: `LocalMind CLI`. Access: `Developer`. Download the `.p8` file (you can only download it once — save it to `~/.appstoreconnect/AuthKey_<KEY_ID>.p8`).

Note the Key ID (10 chars) and Issuer ID (a UUID, shown on the API Keys page).

- [ ] **Step 3: Upload via `xcrun altool`.**

```bash
IPA=$(find src-tauri/gen/apple/build -name "*.ipa" -mtime -1 | head -1)
xcrun altool --upload-app \
  --type ios \
  --file "$IPA" \
  --apiKey YOUR_KEY_ID \
  --apiIssuer YOUR_ISSUER_ID
```

Expected output ends with `UPLOAD SUCCEEDED`. The upload takes 5–15 min depending on bandwidth. If it fails with `Invalid binary`, read the error — common causes: missing privacy strings (Task 3 not done), wrong Team ID in provisioning profile (Task 1 Step 5/6 mismatch).

- [ ] **Step 4: Wait for App Store Connect to process the build.** Visit https://appstoreconnect.apple.com/apps → LocalMind → TestFlight tab. The new build appears with status "Processing" (~10–30 min) → "Ready to Submit" or "Missing Compliance".

If "Missing Compliance", click into the build and answer the export-compliance questions (LocalMind doesn't use proprietary encryption — answer **No** to "Does your app use encryption?" then **Yes** to "Does your app qualify for any exemptions?" → **Yes** to "Your app uses encryption only in ways exempt from export compliance").

- [ ] **Step 5: Install via TestFlight on your phone.**

Open TestFlight on your iPhone (install from App Store if you don't have it). Tap `+` next to LocalMind → install. The "Untrusted Developer" prompt does NOT appear — TestFlight builds are pre-trusted.

Launch LocalMind. Connect screen appears, just like in Task 8 but now via the production-grade install path.

- [ ] **Step 6: Tag and commit a marker for M1 completion.**

```bash
git tag -a m1-ios-bootstrap-complete -m "M1 ships: first TestFlight build of LocalMind iOS"
```

**Exit criteria for Task 9 (and M1 overall):** LocalMind is installed on your physical iPhone via TestFlight and displays the Connect screen.

---

## Task 10 (stretch): GitHub Actions CI for TestFlight uploads

**Context:** Up to Task 9 we've been uploading manually. Automating it on push-to-`main` means M2–M6 development gets continuous TestFlight builds without thinking about it.

**This task is M1 stretch.** If Tasks 1–9 took the full week, ship M1 without it and revisit in M2.

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

Mapping spec §8 M1 exit criteria to plan tasks:

- "Apple Developer setup" → Task 1 ✓
- "`tauri ios init` already done, fix capabilities + Info.plist additions" → Tasks 3, 4 ✓
- "first signed TestFlight build that launches WKWebView and shows existing Connect screen" → Tasks 8, 9 ✓
- "Exit criteria: you can install via TestFlight on your phone and see the React app" → Task 9 Step 5 ✓

Mapping spec §5 ("Bundle layout", "Native modules", "Info.plist additions") subset that M1 must cover:

- Main `LocalMind.app` bundle ID `com.localmind.app` → Task 1 Step 4 ✓
- iOS minimum 15.0 → Task 2 ✓
- All five Info.plist privacy/ATS keys → Task 3 ✓
- iOS-only Rust module skeleton → Task 5 ✓
- `npm run tauri ios build --release` pipeline → Task 8 ✓
- CI / TestFlight automation → Task 10 (stretch) ✓

Deferred to later milestones (correctly out of M1 scope):

- Native Swift modules (QR, mDNS, Keychain, etc.) → M2/M3/M4/M5
- Share extension target → M3
- `apns_pusher` Rust crate → M5
- Server-side endpoints (`/api/pair/apns`, `/api/chats/{id}/resume`) → M4/M5
- Mobile UI breakpoints → M3
- Privacy manifest + App Store metadata → M6

No spec requirement is silently dropped.

---

## What ships after this plan

After Task 9 completes:

- A signed LocalMind iOS app is installable via TestFlight on your iPhone.
- It hosts the existing React UI inside WKWebView and shows the Connect screen.
- It does NOT yet pair with the desktop (no QR scanner, no mDNS browser — that's M2).
- It does NOT yet have the native polish (no FaceID, no share extension, no mobile breakpoints — that's M3).
- The build pipeline is validated end-to-end, so M2 starts on a known-good baseline.

After Task 10 (if completed):

- Every push to `main` produces a TestFlight build automatically.

The next plan to write: `docs/superpowers/plans/<date>-tauri-mobile-ios-m2-pairing.md`, written when this plan completes so it reflects what we actually learned during M1.
