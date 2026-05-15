# Tauri Mobile (iOS) — Parity with the PWA + Native Upgrades

**Status:** Approved design, ready for implementation planning
**Date:** 2026-05-11
**Branch this targets:** new feature branch, off `main` after `feature/voice-diarization` merges
**Scope:** iOS only. Android deferred to a separate brainstorm.

## 1. Goal & Non-Goals

### Goal

Ship a native iOS app on the App Store that delivers everything today's PWA does (chat over LAN to the desktop host) plus first-class native UX:

- Zero-typing pairing via native QR scan + mDNS host discovery
- iOS share-sheet input (text/image → chat composer)
- FaceID/TouchID gate on chat history
- APNs push when the desktop has a reply ready
- Stream resumption that handles backgrounding cleanly

### Non-goals

- On-device inference. The phone is a thin client; llama.cpp does not run on iOS.
- Model management, RAG ingest, image generation. These stay desktop-only, matching the existing "Phone / tablet (PWA)" boundary in `README.md`.
- PWA changes. The PWA continues to exist as the Android story until that gets its own work.
- Cross-device chat sync (multiple phones paired to one desktop).
- Android v1.

## 2. Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  iOS app (Tauri 2 mobile + WKWebView + Swift bridge)         │
│                                                              │
│   React UI (existing, responsive)  ─────► Tauri JS bridge    │
│                                                  │           │
│   iOS-only native modules (Swift) ◄──────────────┘           │
│   ├─ QR scanner (AVFoundation)                               │
│   ├─ mDNS browser (NWBrowser)                                │
│   ├─ Keychain token store                                    │
│   ├─ Biometric gate (LocalAuthentication)                    │
│   ├─ APNs registration + delivery handler                    │
│   ├─ Share extension target (separate app extension)         │
│   └─ Foreground lifecycle (NWPathMonitor; no background     │
│      mode entitlement — see §4)                             │
└──────────────────────────────────────────────────────────────┘
                              │  HTTP + SSE over LAN
                              ▼
┌──────────────────────────────────────────────────────────────┐
│  Desktop host (existing localmind server, additive changes)  │
│                                                              │
│  Axum routes (existing /chat, /events, etc.)                 │
│   + NEW  /pair/apns           — receive phone's APNs token   │
│   + NEW  /chats/{id}/resume   — resume an in-flight stream   │
│   + NEW  apns_pusher (Rust)   — outbound TLS to APNs gateway │
│                                                              │
│  mDNS service `_localmind._tcp` (new, distinct from          │
│  Synapse's existing `_localmind-synapse._tcp`)               │
└──────────────────────────────────────────────────────────────┘
```

### What's new vs. today

- **Frontend:** responsive breakpoints + a handful of iOS-conditional components (~600 LOC additions, ~50 LOC modifications).
- **Tauri command surface:** 7 new iOS-only commands behind `#[cfg(target_os = "ios")]`.
- **Native Swift code:** 6 modules + 1 share extension target.
- **Desktop server:** 3 new endpoints + 1 new Rust crate (`apns_pusher`).
- **Build pipeline:** TestFlight + App Store submission via `npm run tauri ios build`, separate signing config from desktop.

The existing React `src/` bundle runs unchanged inside WKWebView — no UI port for the chat surface itself.

## 3. Pairing, Discovery & Transport

### First-launch flow (zero-typing)

1. App opens → Connect screen → "Find host" button.
2. iOS prompts for Local Network access (iOS 14+ requirement). Without it, mDNS silently returns empty.
3. `NWBrowser` browses `_localmind._tcp` → list of hosts on LAN appears with hostname + version.
4. User taps a host → app prompts "Scan QR".
5. AVFoundation camera scan → extract PIN.
6. POST `/pair` → receive token.
7. Token stored in Keychain.

Subsequent launches: read token from Keychain, skip the Connect screen entirely.

### mDNS service

New service type `_localmind._tcp` advertised by the desktop's chat server. Distinct from Synapse's existing `_localmind-synapse._tcp` (which is for compute pipelining) — keeps the two concerns cleanly separated.

TXT record fields:

- `hostname=<machine>`
- `port=<n>`
- `version=<semver>`
- `host_url=http://<ip>:<port>`

Phone's `Info.plist` declares `_localmind._tcp` under `NSBonjourServices`. Synapse's compute-discovery service is not needed on phone in v1 (chat-only client); if a later version exposes Synapse status on phone, add it then.

### QR payload

JSON, base64-URL-encoded:

```json
{"url":"http://192.168.1.x:port","pin":"123456","v":1}
```

Versioned so future fields don't break older apps. The desktop's existing pairing screen already generates a QR; we adopt this JSON-not-string format.

### Token storage

iOS Keychain via `Security` framework with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`. Survives reboots and app updates; not synced to iCloud. Wrapped behind Tauri commands `keychain_store_token` / `keychain_load_token` implemented in Swift (~80 LOC; no existing Tauri plugin covers Keychain with the access flags we need).

### ATS configuration

`Info.plist` → `NSAppTransportSecurity` → `NSAllowsLocalNetworking = true`. This single key permits HTTP to RFC1918 / `.local` addresses without per-host exceptions. No desktop-side change needed. Public-IP HTTP stays blocked — the desktop is never on a public IP.

### Streaming transport

Existing `text/event-stream` (SSE) endpoints. Reuse the JS SSE client in `src/lib/api.ts` — runs unchanged inside WKWebView. Native code enters the picture only when the app backgrounds (Section 4).

### Failure modes & UI

- No Local Network permission → red banner "Allow Local Network in Settings →".
- No hosts found in 5s of browsing → fall back to manual URL entry (today's PWA flow).
- QR scan fails / wrong PIN → desktop returns 401; phone shows "Pairing failed, regenerate QR on desktop".
- Token rejected on later launch → silently fall back to Connect screen with a banner.

## 4. Push & Background Lifecycle

### The reality

iOS does not permit arbitrary persistent TCP/SSE connections in background. When the app backgrounds, it gets ~30 seconds of execution time, then iOS reclaims the socket. There is no entitlement to escape this — `audio`, `voip`, and `location` background modes are App Review tripwires for a chat app.

"Background streaming continuity" is therefore a *combination* of two mechanisms:

- **APNs** covers the "you walked away" case.
- **Stream resumption** covers the "you briefly tabbed away" case.

Neither alone is sufficient; together they cover the spectrum.

### APNs end-to-end

**Apple Developer setup (one-time).** Generate a `.p8` token-based auth key in App Store Connect. Stored on the desktop as `~/Library/Application Support/LocalMind/data/apns/AuthKey_<KEY_ID>.p8` plus team-id + bundle-id config.

**Phone-side registration.** First launch after pairing:

1. `UNUserNotificationCenter.requestAuthorization`.
2. If granted, `UIApplication.registerForRemoteNotifications`.
3. `didRegisterForRemoteNotificationsWithDeviceToken` callback fires.
4. POST `/pair/apns` with `{token, env: "dev"|"prod"}`.
5. Desktop persists per-paired-device in `data/apns_tokens.json`.

**Desktop-side pusher.** New Rust crate `apns_pusher` using `reqwest` for HTTP/2 to `api.push.apple.com:443` (prod) / `api.sandbox.push.apple.com:443` (TestFlight). JWT auth signed with ES256 against the `.p8` key, regenerated every ~50 min (1hr max per Apple). One persistent HTTP/2 connection pool serves all phones paired to this desktop.

**When to push.** Server hook: chat completion event fires → check if any paired phone is *not* currently subscribed to the chat's SSE stream → push only to those phones. Avoids duplicate "your reply" buzz when the phone is already foregrounded.

**Payload (under the 4 KB APNs limit):**

```json
{
  "aps": {
    "alert": { "title": "Reply ready", "body": "<first 80 chars>" },
    "sound": "default",
    "mutable-content": 1
  },
  "chat_id": "<uuid>",
  "cursor": "<event-id>"
}
```

Tap → app foregrounds → reads `chat_id` from launch options → routes to that chat → hits `/chats/<id>/resume?from=<cursor>` to fetch missed deltas.

### Stream resumption protocol

- Existing SSE endpoint emits events with monotonic `event-id` per chat (e.g. `id: 0`, `id: 1`, ...).
- Server retains the per-chat completion buffer for `RESUME_TTL = 10min` after the last subscriber disconnects (or until the response finishes naturally).
- New endpoint `GET /chats/<id>/resume?from=<cursor>`:
  - If reply still in-flight: SSE stream replaying events `> cursor`, then live events until completion.
  - If reply completed within `RESUME_TTL`: replay-and-close.
  - If buffer evicted: `404 {"reason":"expired"}` → phone shows "Reply finished while you were away" with a "Fetch result" button hitting the regular chat history endpoint.

Phone captures the resume cursor in two places: the last `event-id` seen by the SSE client before disconnect, and the cursor field of the APNs payload.

### Foreground / background transitions on phone

- **Backgrounding:** SSE client persists last `event-id` to disk via `UserDefaults` (non-secret cursor state — Keychain is overkill).
- **Foregrounding:**
  - For each chat with an in-flight reply at background time → call `/chats/<id>/resume?from=<cursor>` automatically.
  - UI shows a thin shimmer on the chat row during the gap.
  - If APNs fired and user tapped, routing already takes them straight to the right chat.
- **Network.framework `NWPathMonitor`** watches LAN connectivity → reconnect attempts on `.satisfied` transitions; exponential backoff; surfaces "Reconnecting…" banner.

### Battery and review

- No background-mode entitlement requested in v1 — review-safe.
- APNs push is "remote notification" only (not silent / not VoIP) — no special entitlement beyond Push Notifications capability.
- No persistent connections held in background — no battery complaints.

## 5. iOS Surfaces & Build Pipeline

### Bundle layout

- `LocalMind.app` (main, bundle ID `com.localmind.app`) — Tauri-generated host for WKWebView + React UI.
- `LocalMindShare.appex` (share extension target, bundle ID `com.localmind.app.share`) — separate process invoked from iOS share sheet.
- App Group `group.com.localmind.shared` — shared container for the share extension to write incoming payloads that the main app picks up on next foreground.

### Native modules (Swift)

| Module | Trigger | Tauri command surface |
|---|---|---|
| `QRScanner` | `AVCaptureSession` + `VNDetectBarcodesRequest` | `ios_scan_qr() -> { url, pin } \| Cancelled` |
| `MDNSBrowser` | `NWBrowser` on `_localmind._tcp` | `ios_browse_hosts(timeout_ms) -> [Host]`, event stream `ios:host-found` |
| `Keychain` | Security framework + `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` | `keychain_store(key, value)`, `keychain_load(key)`, `keychain_delete(key)` |
| `Biometric` | `LocalAuthentication` (`LAContext`) | `biometric_unlock(reason) -> bool` |
| `PushBridge` | `UNUserNotificationCenter` + `registerForRemoteNotifications` | `push_register() -> apns_token`, event `push:tapped` carrying `{chat_id, cursor}` |
| `LifecycleBridge` | `UIApplicationDelegate` willEnterForeground/didEnterBackground + `NWPathMonitor` | event stream `app:lifecycle` (`foreground` \| `background` \| `network-up` \| `network-down`) |

All commands gated `#[cfg(target_os = "ios")]` in `src-tauri/src/lib.rs`. Desktop builds ignore them entirely.

### Share extension

Separate Xcode target. Receives `NSExtensionContext` with text or image attachments. Persists to App Group container at `shared/inbox/<uuid>.json`. Main app on next foreground scans the inbox dir, surfaces items as "Shared with LocalMind →" cards above the composer, user taps to send into the active chat. ~120 LOC of Swift + a `MainInterface.storyboard` for the "Send to LocalMind" confirmation sheet.

### Tauri Mobile config (`tauri.conf.json`)

```json
{
  "bundle": {
    "iOS": {
      "developmentTeam": "ABCD1234EF",
      "minimumSystemVersion": "15.0"
    }
  }
}
```

iOS 15 minimum — covers ~97% of installed base, gives modern Swift Concurrency and `NWBrowser` stability. New `capabilities/ios.json` for iOS-only command allow-list.

### Info.plist additions

- `NSMicrophoneUsageDescription` (voice input — reuse string from desktop)
- `NSCameraUsageDescription` (QR scan)
- `NSLocalNetworkUsageDescription` — "LocalMind connects to your computer running LocalMind over the local network"
- `NSBonjourServices` — `["_localmind._tcp"]`
- `NSAppTransportSecurity` → `NSAllowsLocalNetworking = true`
- `UIBackgroundModes` deliberately *empty* (no background entitlements)

### Build pipeline

- `npm run tauri ios build --release` produces signed `.ipa` (provisioning + signing config from `tauri.conf.json` + the developer team).
- Tauri regenerates the Xcode project on each build. The share extension target is added manually once and tracked in `scripts/inject-share-extension.rb` — an idempotent xcodeproj-mutator script that runs as a Tauri build hook. (Tauri 2 mobile does not yet have first-class multi-target support — this is the accepted workaround.)
- CI: GitHub Actions runner with macOS, fastlane-installed Xcode 16, secrets for `.p8` APNs key + signing cert + provisioning profile. Build → TestFlight upload on every push to `main`; manual promotion to App Store.

### Apple Developer setup (one-time, ~1 day)

- $99/yr Apple Developer Program membership
- Register app ID `com.localmind.app` with Push Notifications + App Groups capabilities
- Register extension app ID `com.localmind.app.share` with App Groups
- Generate `.p8` APNs auth key; save key-id + team-id + bundle-id
- Distribution provisioning profile (App Store)
- Development provisioning profile (TestFlight + on-device debugging)

### App Store review angles to address proactively

- **Guideline 4.2 (minimum native value):** emphasize native QR scan, mDNS discovery, share extension, biometric, push, Keychain — not a thin webview wrapper.
- **Guideline 4.3 (spam):** this is a client for a user's own self-hosted LLM, not a re-skin of ChatGPT.
- **Guideline 5.1.2 (data collection):** zero data collection. Ship a privacy manifest declaring no tracking, no analytics, no third-party SDKs.
- **App Privacy → Network usage:** "Local Network — pairing and chat communication with the user's own desktop."

## 6. UI Strategy

Keep the existing React tree. Don't fork.

- Add a `useIsMobile()` hook + Tailwind responsive breakpoints (`md:` for desktop-only chrome like the multi-column sidebar; mobile uses a drawer).
- Composer gets a thumb-friendly variant on narrow viewports (larger tap targets, mic button next to send, attachment menu collapsed into a `+`).
- Mobile-conditional components live in `src/components/mobile/` — opt-in, zero churn for desktop.

Total React diff estimated ~600 LOC additions, ~50 LOC modifications.

**Voice input on phone.** Record-and-upload: phone captures audio via WKWebView `MediaRecorder` (already proven on desktop), hits the existing voice endpoint on desktop, gets diarized transcript back. No new mobile audio logic needed for v1.

**Chat history caching on phone.** Read-only mirror via existing `/chats` endpoint, persisted to IndexedDB. Lets the user scroll past chats offline (e.g. on the subway). Sync direction is one-way phone-reads-desktop; phone never originates history edits. Fits inside M4.

## 7. Testing

- **Native module unit tests (Swift, XCTest):** QR payload parser, keychain wrapper, mDNS host record decoder, APNs payload builder. Run on macOS CI.
- **Rust unit tests (existing pattern):** APNs JWT generator, resume-cursor logic, retention TTL for the stream buffer. Run on every CI.
- **React component tests (existing vitest setup):** mobile composer rendering, drawer behavior, share-inbox card. Mock the Tauri bridge via `src/test/setup.ts`.
- **Manual smoke matrix per release:** TestFlight build run through pairing via mDNS, pairing via QR fallback, send chat, force-quit + reopen mid-stream (resume), background app + receive APNs, FaceID gate, share text from Notes app, no-LAN graceful degradation.
- **Out of scope for v1:** end-to-end iOS UI tests (XCUITest). Too brittle for solo maintenance against a webview-hosted UI.

## 8. Phased Milestones (6–8 weeks)

| # | Milestone | Calendar | Exit criteria |
|---|---|---|---|
| M1 | Bootstrap & build | Week 1 | TestFlight build installs on your phone and shows existing React app |
| M2 | Pairing & discovery | Weeks 1–2 | Pair phone to desktop with zero typing |
| M3 | Native polish | Week 3 | Mobile responsive UI, biometric gate, share extension, mobile composer |
| M4 | Stream resumption + history cache | Week 4 | Background mid-reply, foreground, no lost output; offline history scroll |
| M5 | APNs | Weeks 5–6 | Lock phone mid-reply, receive push when reply done, tap → opens right chat |
| M6 | App Store submission | Weeks 7–8 | Live on App Store |

## 9. Risks & Mitigations

- **Tauri 2 mobile maturity.** Framework hit 2.0 in Sep 2024; iOS is officially supported but rough around the edges. Mitigations baked in: minimal native bridge (Swift added directly to `gen/apple/` rather than via Tauri plugins), share-extension via manual xcodeproj injection, share inbox via App Group filesystem rather than IPC.
- **App Store rejection on Guideline 4.2.** Real risk — Apple has gotten stricter about webview-wrappers. Mitigation: native-modules-shipping-real-features story is genuine. Appeal language ready emphasizing the native bridges (mDNS, push, biometric, share extension). Worst case adds 1–2 weeks.
- **APNs delivery in real-world conditions.** Desktop must be internet-connected to reach `api.push.apple.com`. If the user's desktop is on a Wi-Fi that blocks outbound 443, push silently breaks. Mitigation: surface APNs reachability on the desktop's pair page; if unreachable, tell the user push won't work for that desktop and degrade gracefully to resume-on-foreground only.
- **TestFlight expiry (90 days).** Builds expire. Mitigation: CI publishes a fresh TestFlight build on every push to `main`; document the 90-day in `CONTRIBUTING.md`.
- **iOS 15 minimum cuts off iPhone 6s and earlier.** Acceptable — those devices can keep using the PWA.

## 10. Open Decisions (closed during brainstorm)

- Voice input: **record-and-upload** through existing diarization endpoint. No on-device pipeline.
- Chat history caching on phone: **read-only IndexedDB mirror**, one-way sync from desktop.
