use std::path::PathBuf;

pub fn app_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| dirs::home_dir().unwrap().join(".localmind"));
    let dir = base.join("LocalMind");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn models_dir() -> PathBuf {
    let dir = app_dir().join("models");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn bin_dir() -> PathBuf {
    let dir = app_dir().join("bin");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn data_dir() -> PathBuf {
    let dir = app_dir().join("data");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn llama_server_path() -> PathBuf {
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    bin_dir().join(name)
}

pub fn rpc_server_path() -> PathBuf {
    let name = if cfg!(windows) {
        "rpc-server.exe"
    } else {
        "rpc-server"
    };
    bin_dir().join(name)
}

pub fn sd_binary_path() -> PathBuf {
    let name = if cfg!(windows) { "sd.exe" } else { "sd" };
    bin_dir().join(name)
}

pub fn sd_output_dir() -> PathBuf {
    let dir = data_dir().join("sd_out");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Path to the persisted Synapse worker token. Generated on first worker start
/// and reused across restarts so the same machine keeps the same identity to
/// all hosts that have already paired with it.
pub fn synapse_token_path() -> PathBuf {
    data_dir().join("synapse-token.txt")
}

/// Phase 4 chunk N: paths to the worker's persisted self-signed TLS cert
/// (PEM) and matching private key. Generated alongside the token on first
/// worker start. Together with the token they form the worker's identity.
pub fn synapse_cert_path() -> PathBuf {
    data_dir().join("synapse-cert.pem")
}

pub fn synapse_key_path() -> PathBuf {
    data_dir().join("synapse-key.pem")
}
