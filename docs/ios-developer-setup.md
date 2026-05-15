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

The free Personal Team's 10-character Team ID isn't shown directly in the Accounts pane until Xcode has provisioned a signing certificate. Easiest way to surface it:

```bash
open src-tauri/gen/apple/localmind.xcodeproj
```

In Xcode:
1. Click the blue `localmind` project at the top of the left sidebar.
2. **TARGETS** → `localmind_iOS`.
3. **Signing & Capabilities** tab.
4. **Team** dropdown → pick "Your Name (Personal Team)".

Xcode auto-provisions a cert. The 10-character Team ID then shows in the **Signing Certificate** row (e.g. `Apple Development: yourname@example.com (ABCD1234EF)`).

Alternatively, after Xcode has provisioned the cert:

```bash
security find-identity -v -p codesigning | grep -i "apple development"
```

The 10 characters in parentheses are your Team ID.

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
