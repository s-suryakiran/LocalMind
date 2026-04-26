# Contributing to LocalMind

Thanks for taking the time to contribute! This document covers everything you need to make a pull request that we can merge quickly.

- [Ground rules](#ground-rules)
- [Setting up a dev environment](#setting-up-a-dev-environment)
- [Project layout](#project-layout)
- [Coding style](#coding-style)
- [Running checks locally](#running-checks-locally)
- [Submitting a pull request](#submitting-a-pull-request)
- [Reporting bugs](#reporting-bugs)
- [Suggesting features](#suggesting-features)
- [Where to start](#where-to-start)

## Ground rules

1. **Be kind.** This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
2. **Open an issue first** for anything bigger than a typo or one-line bug fix. Saves you wasted work if we already have a plan.
3. **Keep PRs small and focused.** One concern per PR. Easier to review = faster to merge.
4. **No new dependencies without justification** — both npm and cargo. State the why in the PR description.

## Setting up a dev environment

You need:

- **Node.js 18+** and **npm**
- **Rust stable** ([rustup.rs](https://rustup.rs))
- Platform toolchain:
  - macOS: `xcode-select --install`
  - Linux: `webkit2gtk` and `build-essential` (or your distro's equivalent)
  - Windows: WebView2 (preinstalled on Win 11) and the MSVC C++ build tools

Then:

```bash
git clone https://github.com/<your-fork>/LocalMind.git
cd LocalMind
npm install
npm run tauri dev
```

The first run will download a llama.cpp release for your platform (~30–100 MB). After that, grab a model from the Marketplace tab.

## Project layout

```
src/                # React frontend (TypeScript, Tailwind v4, Zustand)
  pages/            # Top-level views — one file per route
  components/       # Reusable UI
  lib/              # store, api, types, util
src-tauri/
  src/              # Rust backend (Tauri 2 commands + Axum LAN server)
  icons/            # generated platform icons
public/             # PWA manifest + icons (copied to dist/ on build)
assets/             # icon source + build script
```

Backend modules:

- `llama.rs` — orchestrates child `llama-server` processes for chat + embeddings
- `binaries.rs` — downloads and unpacks llama.cpp + stable-diffusion.cpp on first use
- `models.rs` — Hugging Face search, downloads, on-disk listing
- `server.rs` — Axum LAN server, PIN/token auth, dev proxy to Vite
- `rag.rs` — document chunking, embedding, retrieval
- `sd.rs` — `sd` CLI orchestration for image generation

## Coding style

We rely on each language's standard formatter — please run them before pushing:

### Frontend (TypeScript / React)

- **No global linter is enforced yet** (we'd welcome a PR adding ESLint!).
- We do enforce `tsc --noEmit` in CI — your code must type-check cleanly.
- Use functional components with hooks. Avoid class components (the only exception is `ErrorBoundary` in `main.tsx`).
- State management: Zustand store in `src/lib/store.ts`. Don't introduce React Context unless there's a strong reason.
- Fetch / API helpers go in `src/lib/api.ts`. Keep components free of raw `fetch` calls.
- File names: `PascalCase.tsx` for components/pages, `camelCase.ts` for libraries.
- Tailwind classes inline. Use the CSS variables defined in `index.css` (`var(--color-accent)` etc.) so themes stay consistent.

### Backend (Rust)

- Run `cargo fmt --all` before committing. CI fails on unformatted code.
- Run `cargo clippy --all-targets -- -D warnings`. Treat clippy lints seriously.
- Keep panics out of request paths — return `Result` and surface errors via `.map_err(|e| e.to_string())` for Tauri commands.
- Async everywhere. Use `tokio::task::spawn_blocking` for genuinely CPU-bound work; everything else should be `async fn`.
- One module per file. If a module grows past ~400 lines, split it.

### Comments

Comments explain *why*, not *what*. Examples we like:

```rust
// Vite v7's HMR ping is a WebSocket; forwarding it through reqwest returned
// 101 Switching Protocols, which the browser misread as "reconnected" and
// triggered location.reload(). We refuse the upgrade outright instead.
```

```ts
// On the phone we don't pick a model — we use whatever the host has loaded.
const effectiveModelId = remote ? (llama.modelId ?? null) : activeModelId;
```

If a fix exists because of a specific upstream behaviour, name it.

## Running checks locally

Before pushing, run:

```bash
# Frontend type check
npx tsc --noEmit

# Frontend production build (catches issues tsc misses)
npm run build

# Backend check + lint + format
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo check --all-targets
```

CI runs the same checks on every PR.

## Submitting a pull request

1. Fork the repo and create your branch from `main`:
   ```bash
   git checkout -b fix/something-specific
   ```
2. Make focused commits. Conventional Commits-style is welcome but not required:
   - `feat: add llava 1.6 support`
   - `fix(ios-pwa): refuse vite-ping ws upgrade so location.reload doesn't fire`
   - `docs: add screenshots`
3. Run the [local checks](#running-checks-locally).
4. Push to your fork and open a PR against `main`. Fill out the PR template.
5. Be responsive to review comments. We try to give first feedback within a few days.

PRs need:

- A clear description of **what** changed and **why**.
- Manual test notes (steps you took to verify).
- Updated docs/README if the change affects user-facing behaviour.
- All CI checks green.

## Reporting bugs

Before opening an issue:

1. Check existing [issues](https://github.com/s-suryakiran/LocalMind/issues) — yours might already be tracked.
2. Reproduce on `main`.

Use the [Bug Report](.github/ISSUE_TEMPLATE/bug_report.yml) template. Include:

- What you expected vs. what happened
- Reproduction steps
- OS + version, Node version, Rust version
- Hardware info from Settings → Hardware
- Relevant log output (terminal where `tauri dev` is running, plus any `~/Library/Application Support/LocalMind/logs/`)

## Suggesting features

Open a [Feature Request](.github/ISSUE_TEMPLATE/feature_request.yml). For larger ideas, please open a discussion first so we can talk shape before you spend a weekend on it.

## Where to start

Look for issues tagged:

- [`good first issue`](https://github.com/s-suryakiran/LocalMind/labels/good%20first%20issue) — small, well-scoped tasks
- [`help wanted`](https://github.com/s-suryakiran/LocalMind/labels/help%20wanted) — anything we'd love a contributor to take on
- [`platform-linux`](https://github.com/s-suryakiran/LocalMind/labels/platform-linux), [`platform-windows`](https://github.com/s-suryakiran/LocalMind/labels/platform-windows) — Mac is best-tested; help on other platforms is invaluable

Some areas we know we want help with:

- **Chat templates** — Llama 3, Mistral, Phi 3, Qwen 2.5 — make sure each renders correctly via `--jinja`.
- **Marketplace search** — better ranking, model-card preview, "matching mmproj?" auto-pairing.
- **Mobile UX** — virtual keyboard handling, swipe-to-delete conv, pull-to-refresh.
- **Tauri Mobile** — iOS / Android native shells with parity to the PWA.
- **Tests** — there are essentially none. Even a few unit tests for `models.rs` parsing would help.

## Questions?

- Open a [Discussion](https://github.com/s-suryakiran/LocalMind/issues) for design questions.
- Open an [Issue](https://github.com/s-suryakiran/LocalMind/issues) for bugs or concrete features.

Thanks again for helping make LocalMind better.
