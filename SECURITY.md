# Security Policy

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security bugs — that exposes the problem to everyone before it is fixed.

Instead, email **suryakiranbdsk@gmail.com** with:

- A description of the vulnerability and its potential impact
- Steps to reproduce or a proof-of-concept
- Your preferred contact for follow-up

You can expect an acknowledgement within **72 hours** and a fix or mitigation plan within **14 days** for confirmed issues.

## Scope

LocalMind runs entirely on your local machine and LAN. The main areas of concern are:

- **LAN server auth** — PIN / bearer-token bypass
- **Path traversal** — via the static file server or model download paths
- **Binary integrity** — the llama.cpp / stable-diffusion.cpp download pipeline (`binaries.rs`)

Issues in bundled third-party binaries (llama.cpp, stable-diffusion.cpp) should be reported upstream to those projects as well.
