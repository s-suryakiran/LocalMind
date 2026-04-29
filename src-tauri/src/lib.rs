mod binaries;
mod config;
mod hardware;
mod llama;
mod models;
mod rag;
mod sd;
mod server;
mod synapse;
mod synapse_proto;
mod synapse_token;

use llama::{LlamaSettings, LlamaState, LlamaStatus};
use models::{InstalledModel, ModelKind, ModelListing};
use rag::{Document, RagState, RetrievedChunk};
use sd::{SdImage, SdRequest, SdState};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use synapse::{SynapsePeer, SynapseState, SynapseWorkerStatus};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppStateHolder {
    llama: Arc<LlamaState>,
    rag: Arc<RagState>,
    sd: Arc<SdState>,
    synapse: Arc<SynapseState>,
    lan_url: parking_lot_lite::Once<String>,
    pin: String,
    #[allow(dead_code)]
    tokens: Arc<Mutex<HashSet<String>>>,
}

mod parking_lot_lite {
    use std::sync::Mutex;
    pub struct Once<T>(Mutex<Option<T>>);
    impl<T: Clone> Once<T> {
        pub fn new() -> Self {
            Self(Mutex::new(None))
        }
        pub fn set(&self, v: T) {
            *self.0.lock().unwrap() = Some(v);
        }
        pub fn get(&self) -> Option<T> {
            self.0.lock().unwrap().clone()
        }
    }
}

#[tauri::command]
fn detect_hardware() -> hardware::HardwareInfo {
    hardware::detect()
}

#[tauri::command]
async fn search_models(query: String, limit: Option<u32>) -> Result<Vec<ModelListing>, String> {
    models::search_huggingface(&query, limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn download_model(
    app: AppHandle,
    repo: String,
    filename: String,
    kind: Option<String>,
) -> Result<InstalledModel, String> {
    let kind = match kind.as_deref() {
        Some("vision") => ModelKind::Vision,
        Some("embedding") => ModelKind::Embedding,
        Some("whisper") => ModelKind::Whisper,
        Some("sd") => ModelKind::Sd,
        _ => ModelKind::Llm,
    };
    models::download_model(&app, &repo, &filename, kind)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_installed_models() -> Result<Vec<InstalledModel>, String> {
    models::list_installed().map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_model(id: String) -> Result<(), String> {
    models::delete_model(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_llama(
    app: AppHandle,
    state: State<'_, Arc<AppStateHolder>>,
    settings: LlamaSettings,
) -> Result<LlamaStatus, String> {
    state
        .llama
        .start(&app, settings)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_llama(state: State<'_, Arc<AppStateHolder>>) -> Result<(), String> {
    state.llama.stop().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn llama_status(state: State<'_, Arc<AppStateHolder>>) -> Result<LlamaStatus, String> {
    Ok(state.llama.status().await)
}

#[tauri::command]
async fn start_embedding_server(
    app: AppHandle,
    state: State<'_, Arc<AppStateHolder>>,
    model_id: String,
) -> Result<LlamaStatus, String> {
    state
        .llama
        .start_embedding(&app, model_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_embedding_server(state: State<'_, Arc<AppStateHolder>>) -> Result<(), String> {
    state
        .llama
        .stop_embedding()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_lan_url(state: State<'_, Arc<AppStateHolder>>) -> Option<String> {
    state.lan_url.get()
}

#[tauri::command]
fn get_lan_pin(state: State<'_, Arc<AppStateHolder>>) -> String {
    state.pin.clone()
}

#[tauri::command]
async fn ensure_engine(app: AppHandle) -> Result<String, String> {
    binaries::ensure_llama_server(&app)
        .await
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn rag_list(state: State<'_, Arc<AppStateHolder>>) -> Result<Vec<Document>, String> {
    Ok(state.rag.list().await)
}

#[tauri::command]
async fn rag_delete(state: State<'_, Arc<AppStateHolder>>, id: String) -> Result<(), String> {
    state.rag.delete(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn rag_ingest(
    state: State<'_, Arc<AppStateHolder>>,
    path: String,
) -> Result<Document, String> {
    let port = state.llama.embedding_port().await;
    if !state.llama.embedding_running().await {
        return Err("Embedding server is not running. Start one on the Settings page.".into());
    }
    let embedder = rag::Embedder::new(port);
    state
        .rag
        .ingest(std::path::Path::new(&path), &embedder)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn rag_search(
    state: State<'_, Arc<AppStateHolder>>,
    query: String,
    top_k: Option<usize>,
    doc_ids: Option<Vec<String>>,
) -> Result<Vec<RetrievedChunk>, String> {
    if !state.llama.embedding_running().await {
        return Ok(vec![]);
    }
    let port = state.llama.embedding_port().await;
    let embedder = rag::Embedder::new(port);
    let emb = embedder
        .embed_one(&query)
        .await
        .map_err(|e| e.to_string())?;
    Ok(state
        .rag
        .retrieve(&emb, top_k.unwrap_or(5), doc_ids.as_deref())
        .await)
}

#[tauri::command]
async fn sd_generate(
    app: AppHandle,
    state: State<'_, Arc<AppStateHolder>>,
    request: SdRequest,
) -> Result<SdImage, String> {
    state
        .sd
        .generate(&app, request)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn sd_busy(state: State<'_, Arc<AppStateHolder>>) -> Result<bool, String> {
    Ok(state.sd.is_busy().await)
}

#[tauri::command]
async fn ensure_sd(app: AppHandle) -> Result<String, String> {
    binaries::ensure_sd(&app)
        .await
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_synapse_worker(
    app: AppHandle,
    state: State<'_, Arc<AppStateHolder>>,
    port: Option<u16>,
) -> Result<SynapseWorkerStatus, String> {
    state
        .synapse
        .start_worker(&app, port)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_synapse_worker(state: State<'_, Arc<AppStateHolder>>) -> Result<(), String> {
    state.synapse.stop_worker().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn synapse_worker_status(
    state: State<'_, Arc<AppStateHolder>>,
) -> Result<SynapseWorkerStatus, String> {
    Ok(state.synapse.status().await)
}

#[tauri::command]
async fn restart_synapse_worker(
    app: AppHandle,
    state: State<'_, Arc<AppStateHolder>>,
    port: Option<u16>,
) -> Result<SynapseWorkerStatus, String> {
    state
        .synapse
        .restart_worker(&app, port)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn synapse_list_peers(
    state: State<'_, Arc<AppStateHolder>>,
) -> Result<Vec<SynapsePeer>, String> {
    Ok(state.synapse.list_peers().await)
}

fn generate_pin() -> String {
    let raw = uuid::Uuid::new_v4().as_u128();
    format!("{:06}", (raw % 1_000_000) as u32)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let llama = LlamaState::new();
    let rag = RagState::new();
    let sd = SdState::new();
    let synapse = SynapseState::new();
    let pin = generate_pin();
    let tokens = Arc::new(Mutex::new(HashSet::new()));
    let holder = Arc::new(AppStateHolder {
        llama: llama.clone(),
        rag: rag.clone(),
        sd: sd.clone(),
        synapse: synapse.clone(),
        lan_url: parking_lot_lite::Once::new(),
        pin: pin.clone(),
        tokens: tokens.clone(),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(holder.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            let holder2 = holder.clone();
            let llama2 = llama.clone();
            let pin2 = pin.clone();
            let tokens2 = tokens.clone();
            let synapse2 = synapse.clone();
            let handle_for_discovery = handle.clone();
            tauri::async_runtime::spawn(async move {
                let resource_dir = handle.path().resource_dir().ok();
                let static_dir = resource_dir.map(|d| d.join("dist")).filter(|p| p.exists());
                match server::start_lan_server(llama2, static_dir, 3939, pin2, tokens2).await {
                    Ok(url) => {
                        holder2.lan_url.set(url.clone());
                        let _ = handle.emit("lan:ready", url);
                    }
                    Err(e) => {
                        eprintln!("LAN server failed: {e}");
                    }
                }
            });
            // Kick off mDNS browsing immediately so the Synapse page sees
            // peers as soon as it mounts, without each `useEffect` having to
            // wait for the daemon to spin up.
            tauri::async_runtime::spawn(async move {
                if let Err(e) = synapse2.start_discovery(&handle_for_discovery).await {
                    eprintln!("synapse discovery failed: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_hardware,
            search_models,
            download_model,
            list_installed_models,
            delete_model,
            start_llama,
            stop_llama,
            llama_status,
            start_embedding_server,
            stop_embedding_server,
            get_lan_url,
            get_lan_pin,
            ensure_engine,
            rag_list,
            rag_delete,
            rag_ingest,
            rag_search,
            sd_generate,
            sd_busy,
            ensure_sd,
            start_synapse_worker,
            stop_synapse_worker,
            synapse_worker_status,
            restart_synapse_worker,
            synapse_list_peers,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
