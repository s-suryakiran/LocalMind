# iOS developer setup

One-time configuration for building LocalMind on iOS using **free Xcode "Personal Team" signing** (no paid Apple Developer Program required through milestones M1–M4).

## 1. Install Xcode

Full Xcode (not just Command Line Tools) from the Mac App Store.

```bash
xcode-select -p
# Expected: /Applications/Xcode.app/Contents/Developer
# If you see /Library/Developer/CommandLineTools, switch:
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
```

## 2. Sign Xcode into your Apple ID

Xcode → **Settings… (⌘,)** → **Accounts** → `+` → **Apple ID** → enter Apple ID + password.

A "Personal Team" row appears under your Apple ID once you sign in.

## 3. Get your Team ID

The free Personal Team's 10-character Team ID isn't shown directly in the Accounts pane until Xcode has provisioned a signing certificate. Surface it by letting Xcode generate the provisioning profile:

```bash
open src-tauri/gen/apple/localmind.xcodeproj
```

In Xcode:
1. Click the blue `localmind` project at the top of the left sidebar.
2. **TARGETS** → `localmind_iOS`.
3. **Signing & Capabilities** tab.
4. **Team** dropdown → pick "Your Name (Personal Team)".

Xcode then writes a `.mobileprovision` file to `~/Library/Developer/Xcode/UserData/Provisioning Profiles/` containing the authoritative Team ID for the team Xcode has account-level access to. Read it out:

```bash
ls ~/Library/Developer/Xcode/UserData/Provisioning\ Profiles/
# pick the most-recent .mobileprovision (or the one named after com.localmind.app)
security cms -D -i ~/Library/Developer/Xcode/UserData/Provisioning\ Profiles/<UUID>.mobileprovision \
  | grep -A1 TeamIdentifier
```

Look for `<key>TeamIdentifier</key>` → the first `<string>` after it is your 10-character Team ID. Also useful nearby: `TeamName` (your real name).

> ⚠️ **Don't read the Team ID from `security find-identity`.** Keychain certs can be stale (left over from a previously-revoked Personal Team), and the cert's "Apple Development: ... (XXXXXXXXXX)" common name may show a Team ID that Xcode no longer has account-level access to. This was a real M1 debugging trap — see the M1 plan's "Discovered during M1" notes. The provisioning profile is the source of truth because Xcode regenerates it from the live Apple ID, not from cached keychain state.

## 4. Create your local Tauri config override

Per-developer Tauri config overrides are gitignored (this is a public repo, and each contributor has their own free Team ID).

Create `src-tauri/tauri.ios.local.conf.json`:

```json
{
  "bundle": {
    "iOS": {
      "developmentTeam": "ABCD1234EF"
    }
  }
}
```

Replace `ABCD1234EF` with the Team ID from step 3.

## 5. Trust your iPhone for development

Plug your iPhone in via USB, unlock it, tap **Trust** when prompted.

```bash
xcrun xctrace list devices 2>&1 | grep -i iphone | head -3
# Expected: your iPhone listed with a UDID
```

In Xcode → **Window → Devices and Simulators** → select your iPhone in the sidebar; wait for the status dot to go yellow → green (~30 s the first time).

## 6. Build

Two convenience scripts are wired up in `package.json`:

```bash
npm run ios:dev     # build + install on Simulator or connected device, with hot reload
npm run ios:build   # release build (.ipa); only useful with paid signing
```

Both pass `--config src-tauri/tauri.ios.local.conf.json` to merge in your Team ID at build time.

## Caveats of the free-signing path

- App expires every **7 days**. Re-run `npm run ios:dev` to re-sign.
- Max **3 free-signed apps per device** at a time.
- No TestFlight, no App Store, no APNs push, no App Groups (share extension).

These limits go away when (and only when) the project enrolls in the paid Apple Developer Program — deferred to M5 (push) or M6 (App Store), whichever comes first. See [`docs/superpowers/plans/2026-05-11-tauri-mobile-ios-m1-bootstrap.md`](superpowers/plans/2026-05-11-tauri-mobile-ios-m1-bootstrap.md) for the cost-deferral rationale.

## First-launch trust prompt on device

The first time a free-signed app from a new Apple ID is installed on the iPhone, iOS treats the developer as untrusted:

iPhone → **Settings → General → VPN & Device Management** → tap your Apple ID under **DEVELOPER APP** → **Trust "Apple Development: yourname@..."** → **Trust**.

After this, re-launch LocalMind from the home screen. You only do this once per Apple ID per device.
