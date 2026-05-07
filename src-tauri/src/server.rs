use crate::llama::LlamaState;
use crate::synapse::SynapseState;
use anyhow::Result;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{any, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower_http::{cors::CorsLayer, services::ServeDir};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub llama: Arc<LlamaState>,
    pub http: reqwest::Client,
    pub pin: String,
    pub tokens: Arc<Mutex<HashSet<String>>>,
    /// Phase 4 chunk P: SynapseState handle so the LAN server can serve a
    /// read-only view of the worker + discovered peers to paired phones.
    pub synapse: Arc<SynapseState>,
}

#[derive(Deserialize)]
struct PairRequest {
    pin: String,
}

#[derive(Serialize)]
struct PairResponse {
    token: String,
}

pub async fn start_lan_server(
    llama: Arc<LlamaState>,
    static_dir: Option<PathBuf>,
    port: u16,
    pin: String,
    tokens: Arc<Mutex<HashSet<String>>>,
    synapse: Arc<SynapseState>,
) -> Result<String> {
    let state = AppState {
        llama,
        http: reqwest::Client::new(),
        pin,
        tokens,
        synapse,
    };

    let mut app = Router::new()
        .route("/api/health", get(health))
        .route("/api/pair", post(pair))
        .route("/api/status", get(status))
        // Phase 4 chunk P: read-only Synapse views for paired phones/iPads.
        // The phone can't actually drive the worker (start/stop, edit
        // tokens, change splits) — those stay desktop-only — but it can
        // observe everything the desktop sees.
        .route("/api/synapse/status", get(synapse_status))
        .route("/api/synapse/peers", get(synapse_peers))
        .route("/api/synapse/sessions", get(synapse_sessions))
        // Phase 4 chunk R: Prometheus-format metrics. Plain text scrape
        // target at /metrics, no client lib dep — the format is small
        // enough to hand-roll. Auth still applies via the same Bearer
        // gate as the rest of /api, so a paired Prometheus server reaches
        // it but a drive-by scrape doesn't.
        .route("/api/metrics", get(prometheus_metrics))
        .route("/v1/*rest", any(proxy_v1))
        .route("/health", get(proxy_health))
        // Vite's HMR client polls this when its WebSocket drops; on a 200 it
        // calls `location.reload()`. Our proxy can't carry WebSockets, so the
        // socket is permanently down — we must keep returning a non-success
        // here or the phone reload-loops.
        .route(
            "/__vite_ping",
            get(|| async { (StatusCode::SERVICE_UNAVAILABLE, "no-hmr") }),
        )
        .nest_service("/sd-images", ServeDir::new(crate::config::sd_output_dir()));

    if let Some(dir) = static_dir {
        // Production / packaged bundle: serve the prebuilt React app from disk.
        app = app.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true));
    } else {
        // Dev mode (no `dist/`): forward unknown paths to the running Vite dev
        // server so a phone hitting :3939 still loads the React app. HMR
        // websockets won't survive the proxy, but the page renders and chat
        // works end-to-end.
        app = app.fallback(any(proxy_dev_frontend));
    }

    let app = app
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let bound = listener.local_addr()?;

    let ip = local_ip_address::local_ip()
        .map(|i| i.to_string())
        .unwrap_or_else(|_| "127.0.0.1".into());

    let url = format!("http://{}:{}", ip, bound.port());

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("LAN server error: {e}");
        }
    });

    Ok(url)
}

// Endpoints that don't require a Bearer token: discovery + pairing + static assets.
fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/health" | "/api/health" | "/api/pair" | "/manifest.webmanifest"
    ) || (!path.starts_with("/api") && !path.starts_with("/v1") && !path.starts_with("/sd-images"))
}

async fn auth(State(s): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    if is_public_path(&path) {
        return next.run(request).await;
    }
    let token = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").map(str::to_string));
    let ok = match token {
        Some(t) => s.tokens.lock().unwrap().contains(&t),
        None => false,
    };
    if ok {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "pair with the PIN shown on the desktop app",
        )
            .into_response()
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "LocalMind" }))
}

async fn pair(
    State(s): State<AppState>,
    Json(req): Json<PairRequest>,
) -> Result<Json<PairResponse>, (StatusCode, &'static str)> {
    if req.pin.trim() != s.pin {
        return Err((StatusCode::UNAUTHORIZED, "wrong PIN"));
    }
    let token = Uuid::new_v4().to_string();
    s.tokens.lock().unwrap().insert(token.clone());
    Ok(Json(PairResponse { token }))
}

async fn status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let st = s.llama.status().await;
    Json(serde_json::to_value(&st).unwrap())
}

async fn proxy_health(State(s): State<AppState>) -> Response {
    let port = s.llama.status().await.port;
    let url = format!("http://127.0.0.1:{}/health", port);
    forward(
        &s.http,
        &url,
        Request::builder().uri("/").body(Body::empty()).unwrap(),
    )
    .await
}

async fn proxy_dev_frontend(State(s): State<AppState>, req: Request<Body>) -> Response {
    // We can't carry WebSocket frames through reqwest. If we forward an
    // Upgrade request to Vite, Vite returns 101 Switching Protocols and we
    // pipe that back to the browser — the browser then thinks the WebSocket
    // opened, fires `open`, and Vite's ping logic calls location.reload().
    // Refusing the upgrade outright makes the ping `error` and the page stays
    // put. (Cost: no HMR over the LAN proxy, which we don't carry anyway.)
    if req.headers().get(axum::http::header::UPGRADE).is_some() {
        return (StatusCode::NOT_FOUND, "websocket not proxied").into_response();
    }
    // Forward to the Vite dev server on the desktop. This only runs in dev
    // (when no bundled `dist/` exists), so hard-coding the localhost URL is OK.
    let path = req.uri().path().to_string();
    let qs = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let url = format!("http://127.0.0.1:1420{}{}", path, qs);
    forward(&s.http, &url, req).await
}

async fn proxy_v1(State(s): State<AppState>, req: Request<Body>) -> Response {
    let st = s.llama.status().await;
    if !st.running {
        return (StatusCode::SERVICE_UNAVAILABLE, "no model loaded on host").into_response();
    }
    let port = st.port;
    let path = req.uri().path().to_string();
    let qs = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let url = format!("http://127.0.0.1:{}{}{}", port, path, qs);
    forward(&s.http, &url, req).await
}

async fn forward(client: &reqwest::Client, url: &str, req: Request<Body>) -> Response {
    let method = req.method().clone();
    let headers = req.headers().clone();
    let body_bytes = match axum::body::to_bytes(req.into_body(), 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("body error: {e}")).into_response(),
    };

    let mut builder = client.request(method, url);
    for (k, v) in headers.iter() {
        if k == "host" || k == "content-length" || k == "authorization" {
            continue;
        }
        builder = builder.header(k, v);
    }
    builder = builder.body(body_bytes.to_vec());

    let upstream = match builder.send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("upstream error: {e}")).into_response(),
    };

    let status = upstream.status();
    let mut rheaders = HeaderMap::new();
    for (k, v) in upstream.headers().iter() {
        rheaders.insert(k.clone(), v.clone());
    }
    let stream = upstream.bytes_stream();
    let body = Body::from_stream(stream);

    let mut resp = Response::builder().status(status);
    if let Some(h) = resp.headers_mut() {
        h.extend(rheaders);
    }
    resp.body(body).unwrap()
}

// Phase 4 chunk P: Synapse read-only views for the phone PWA.
//
// `/api/synapse/status`  — is the worker running, on what port, what's its PID
// `/api/synapse/peers`   — current discovered-on-LAN peer list
// `/api/synapse/sessions` — live count of authenticated host connections
//
// The phone consumes these to render its read-only Synapse tab. We don't
// expose start/stop/restart from the LAN — the phone is for monitoring,
// not control, and gating mutations behind the desktop UI keeps the
// surface area small.
async fn synapse_status(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.synapse.status().await)
}

async fn synapse_peers(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.synapse.list_peers().await)
}

async fn synapse_sessions(State(_): State<AppState>) -> impl IntoResponse {
    let count = crate::auth_proxy::ACTIVE_SESSIONS.load(std::sync::atomic::Ordering::Relaxed);
    Json(serde_json::json!({ "count": count }))
}

/// Phase 4 chunk R: Prometheus exposition format. Plain text, one metric
/// per line per series, with HELP/TYPE preambles. The contract is small
/// and stable: rename a metric here = breaking change for anyone
/// scraping it, so we treat field names as part of the public API.
async fn prometheus_metrics(State(s): State<AppState>) -> impl IntoResponse {
    use std::fmt::Write as _;
    let mut out = String::new();

    // Worker mode + active sessions.
    let synapse_status = s.synapse.status().await;
    let _ = writeln!(
        out,
        "# HELP localmind_synapse_worker_running 1 if the Synapse worker is currently spawned, 0 otherwise."
    );
    let _ = writeln!(out, "# TYPE localmind_synapse_worker_running gauge");
    let _ = writeln!(
        out,
        "localmind_synapse_worker_running {}",
        if synapse_status.running { 1 } else { 0 }
    );

    let _ = writeln!(
        out,
        "# HELP localmind_synapse_worker_port TCP port of the worker's auth proxy (0 when not running)."
    );
    let _ = writeln!(out, "# TYPE localmind_synapse_worker_port gauge");
    let _ = writeln!(
        out,
        "localmind_synapse_worker_port {}",
        if synapse_status.running {
            synapse_status.port
        } else {
            0
        }
    );

    let active = crate::auth_proxy::ACTIVE_SESSIONS.load(std::sync::atomic::Ordering::Relaxed);
    let _ = writeln!(
        out,
        "# HELP localmind_synapse_active_sessions Authenticated host connections currently held by the auth proxy."
    );
    let _ = writeln!(out, "# TYPE localmind_synapse_active_sessions gauge");
    let _ = writeln!(out, "localmind_synapse_active_sessions {active}");

    // Discovered peers, with a label per verification state. Useful to
    // alert on "discovered: yes, verified: no" — that's "your beacon
    // signing or token-pasting workflow has drifted."
    let peers = s.synapse.list_peers().await;
    let verified_count = peers.iter().filter(|p| p.verified).count();
    let unverified_count = peers.len() - verified_count;
    let _ = writeln!(
        out,
        "# HELP localmind_synapse_peers Discovered peers on the LAN, broken down by verification state."
    );
    let _ = writeln!(out, "# TYPE localmind_synapse_peers gauge");
    let _ = writeln!(
        out,
        "localmind_synapse_peers{{verified=\"true\"}} {verified_count}"
    );
    let _ = writeln!(
        out,
        "localmind_synapse_peers{{verified=\"false\"}} {unverified_count}"
    );

    // llama-server status, for quick "is anything loaded" checks.
    let llama_status = s.llama.status().await;
    let _ = writeln!(
        out,
        "# HELP localmind_llama_chat_running 1 if a chat model is currently loaded, 0 otherwise."
    );
    let _ = writeln!(out, "# TYPE localmind_llama_chat_running gauge");
    let _ = writeln!(
        out,
        "localmind_llama_chat_running {}",
        if llama_status.running { 1 } else { 0 }
    );

    let _ = writeln!(
        out,
        "# HELP localmind_llama_embed_running 1 if an embedding model is currently loaded, 0 otherwise."
    );
    let _ = writeln!(out, "# TYPE localmind_llama_embed_running gauge");
    let _ = writeln!(
        out,
        "localmind_llama_embed_running {}",
        if llama_status.embedding_running { 1 } else { 0 }
    );

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::is_public_path;

    #[test]
    fn sw_js_is_public() {
        assert!(is_public_path("/sw.js"));
    }

    #[test]
    fn register_sw_js_is_public() {
        assert!(is_public_path("/registerSW.js"));
    }

    #[test]
    fn workbox_chunks_are_public() {
        // workbox-window emits hashed chunks like /workbox-abc123.js
        assert!(is_public_path("/workbox-fa3b7a48.js"));
    }

    #[test]
    fn manifest_is_public() {
        assert!(is_public_path("/manifest.webmanifest"));
    }

    #[test]
    fn api_status_still_requires_auth() {
        assert!(!is_public_path("/api/status"));
    }

    #[test]
    fn v1_chat_still_requires_auth() {
        assert!(!is_public_path("/v1/chat/completions"));
    }
}
