<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="128" alt="LocalMind icon" />

# LocalMind

**Run open-source LLMs entirely on your device. Bring the chat to your phone over your home Wi-Fi.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?logo=tauri)](https://tauri.app)
[![React](https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=white)](https://react.dev)
[![llama.cpp](https://img.shields.io/badge/inference-llama.cpp-blueviolet)](https://github.com/ggerganov/llama.cpp)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

[Quickstart](#quickstart) · [Features](#features) · [Architecture](#architecture) · [Contributing](CONTRIBUTING.md) · [Roadmap](#roadmap)

</div>

---

LocalMind is a desktop app that runs open-source language models locally and exposes them as a chat UI you can open in a browser **or install on your phone as a PWA**, all on your private Wi-Fi. Models, conversations, documents, and images never leave your machine.

> **Status:** v0.1, actively developed. Mac (Apple Silicon) is the most-tested platform; Linux/Windows builds compile but receive less testing. Help wanted!

## Features

- 🧠 **Local chat** — streaming responses from any GGUF model via [llama.cpp](https://github.com/ggerganov/llama.cpp). Auto-detects Metal / CUDA / Vulkan and picks the right backend.
- 🛒 **Built-in marketplace** — search and one-click download GGUF models from Hugging Face directly in-app.
- 📚 **RAG over your docs** — drop PDFs / text into the Knowledge tab, get citations alongside answers (uses `nomic-embed-text` by default).
- 👁️ **Vision** — bring your own LLaVA / vision-language model + projector, attach images in chat.
- 🎨 **Image generation** — bundled `stable-diffusion.cpp`; generate locally with FLUX / SD models.
- 🎙️ **Voice** — Web Speech API for input and TTS playback. No cloud round-trip.
- 📱 **Phone PWA** — pair an iPhone or Android with a 6-digit PIN, "Add to Home Screen", chat with the model running on your computer from anywhere on your Wi-Fi.
- 🔒 **Private by default** — embedded Axum LAN server only listens on your local network and requires a paired bearer token for any API call.

## Screenshots

| Chat | Marketplace |
|------|-------------|
| ![LocalMind chat view](assets/screenshots/home_page.png) | ![LocalMind marketplace](assets/screenshots/marketplace.png) |

## Quickstart

### Prerequisites

| Requirement | macOS | Linux | Windows |
|---|---|---|---|
| Node.js 18+ | `brew install node` | distro pkg | [nodejs.org](https://nodejs.org/) |
| Rust (stable) | [rustup.rs](https://rustup.rs) | [rustup.rs](https://rustup.rs) | [rustup.rs](https://rustup.rs) |
| Xcode CLT | `xcode-select --install` | — | — |
| WebView2 | — | — | preinstalled on Win 11 |
| webkit2gtk | — | distro pkg | — |

llama.cpp itself is downloaded automatically on first model load — no manual setup.

### Run in development

```bash
git clone https://github.com/<you>/LocalMind.git
cd LocalMind
npm install
npm run tauri dev
```

The Tauri window opens with a hardware probe and a Marketplace tab to grab a model. Try `qwen2.5-7b-instruct GGUF` for a sensible starter.

### Build a release bundle

```bash
npm run tauri build
```

Outputs platform-native installers under `src-tauri/target/release/bundle/` (.dmg, .msi, .deb, .AppImage).

## Pair a phone

1. **Desktop**: Settings → **Pair a phone or tablet** — shows a LAN URL, a 6-digit PIN, and a QR code.
2. **Phone** (same Wi-Fi): open the URL in Safari/Chrome → enter the PIN → **Connect**.
3. Share sheet → **Add to Home Screen** for an installable PWA.

The phone uses whichever model the desktop has loaded. See the [phone setup notes](#phone--tablet-pwa) for details.

## Architecture

```
┌────────────────────────┐         LAN (HTTP)         ┌──────────────────┐
│  Desktop (Tauri 2)     │                            │  Phone PWA       │
│                        │   /v1/chat/completions     │                  │
│  React UI ◄────────────┤◄───── (bearer token) ──────┤  React UI (same  │
│       │                │                            │   bundle)        │
│       ▼                │   /api/pair                │                  │
│  Tauri command IPC     │   /api/status              │                  │
│       │                │                            │                  │
│       ▼                │                            │                  │
│  Rust backend          │                            │                  │
│  ├─ llama.rs ──────────┤──── spawns ────► llama-server (port 8181)     │
│  ├─ models.rs           │                  llama-server-embed (8182)    │
│  ├─ rag.rs              │                                               │
│  ├─ sd.rs ──────────── ┤──── runs ──────► sd CLI (one-shot per image) │
│  └─ server.rs ──────── ┤──── Axum on 0.0.0.0:3939 ─┘                   │
└────────────────────────┘                                               │
```

- **Frontend** (`src/`) — React 19 + TypeScript + Tailwind v4, Zustand for state, single bundle shared between desktop and PWA.
- **Backend** (`src-tauri/src/`) — Rust, Tauri 2, Axum LAN server. `llama.rs` orchestrates child llama-server processes; `server.rs` proxies `/v1/*` to llama-server and serves the React app for paired phones.
- **Inference engine** — bundled `llama.cpp` and `stable-diffusion.cpp` binaries downloaded per-platform on first use, cached under `~/Library/Application Support/LocalMind/bin/`.

## Project structure

```
LocalMind/
├── src/                      # React frontend
│   ├── pages/                # Chat, Marketplace, Models, Knowledge, ImageGen, Settings, Connect
│   ├── components/           # Sidebar, HardwareBadge, etc.
│   └── lib/                  # store (Zustand), api (Tauri + LAN), types
├── src-tauri/
│   ├── src/                  # Rust backend
│   │   ├── llama.rs          # spawns llama-server children
│   │   ├── binaries.rs       # downloads/extracts llama.cpp + sd
│   │   ├── models.rs         # HF search, download, listing
│   │   ├── rag.rs            # document ingest + embedding search
│   │   ├── sd.rs             # stable-diffusion.cpp orchestration
│   │   └── server.rs         # Axum LAN server + auth
│   ├── icons/                # platform icon bundle
│   └── Cargo.toml
├── public/                   # PWA manifest + icons
├── assets/                   # icon source (SVG) + build script
├── .github/                  # CI workflows + issue/PR templates
├── CONTRIBUTING.md           # how to contribute
└── package.json
```

## Phone / tablet (PWA)

LocalMind ships a chat-only mobile UI as an installable PWA, served from the same LAN endpoint:

1. On the **desktop**, open Settings → **Pair a phone or tablet**. You'll see the LAN URL, a 6-digit PIN, and a QR code that bundles both.
2. On the **phone**, open the LAN URL in Safari (must be Safari for "Add to Home Screen"). The Connect screen appears.
3. Enter the PIN → **Connect**. Token stored locally; subsequent visits skip the Connect screen.
4. Share sheet → **Add to Home Screen**.

Caveats:
- Mobile UI is **chat-only** for now — model management, RAG, and image generation are desktop-side. The phone uses whichever model the host has loaded.
- The PIN regenerates on every desktop start, so paired phones re-pair after a restart.
- Communication is HTTP over your LAN. Treat the local network as the trust boundary.
- Need a hard reset on the phone? Visit `<lan-url>/?reset` in Safari — clears the connection state.

### Going fully native (Tauri Mobile)

```bash
npm run tauri ios init      # requires Xcode
npm run tauri android init  # requires Android Studio + JDK
npm run tauri ios dev
npm run tauri android dev
```

The native build hits the same Connect screen on first launch.

## Contributing

We love PRs. **Start here: [CONTRIBUTING.md](CONTRIBUTING.md)**.

Good first issues:

- New model templates / chat-template support
- Linux/Windows packaging fixes
- Improving the marketplace search ranking
- Translations / i18n
- Lighter mobile UI

If you're not sure where to start, open a discussion or pick anything labeled [`good first issue`](https://github.com/<you>/LocalMind/labels/good%20first%20issue).

## Roadmap

- [ ] Service-worker-backed offline shell for the PWA
- [ ] Tauri Mobile (iOS/Android) reaching parity with the PWA
- [ ] Multi-model concurrent serving (chat + embed + vision in one session)
- [ ] Speaker diarization for voice
- [ ] Plugin system for custom tools

## Troubleshooting

- **"image input is not supported"** — your vision model is missing its mmproj projector. See [Vision models](#vision-llava-models) or download the matching `mmproj-*.gguf` from the same repo.
- **Model output loops on a fragment** — sampler defaults of `repeat_penalty=1.1` and `frequency_penalty=0.3` prevent most loops. If still bad, try a different quant.
- **`llama-server did not become ready`** — usually a port conflict. Stop the model from My-models and try again; orphaned servers on 8181/8182 are auto-killed before respawn.
- **Phone PWA blank** — visit `<lan-url>/?reset` to clear stored state, or remove the home-screen icon and re-add it.

## Security

LocalMind's LAN server requires a paired bearer token for `/api/*` and `/v1/*` routes. The PIN regenerates on every desktop start. **Communication is HTTP over your local Wi-Fi** — anyone on the same network can attempt to pair if they know the PIN. For sensitive use, treat your LAN as the trust boundary.

If you find a security issue, please [report it privately](SECURITY.md) rather than opening a public issue.

## License

[MIT](LICENSE) © LocalMind contributors.

## Acknowledgements

Standing on the shoulders of:

- [llama.cpp](https://github.com/ggerganov/llama.cpp) by Georgi Gerganov
- [stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp) by leejet
- [Tauri](https://tauri.app)
- [Hugging Face](https://huggingface.co) for the model hub
