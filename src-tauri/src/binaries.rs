use crate::{
    config,
    hardware::{self, Accelerator},
};
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub stage: String,
    pub downloaded: u64,
    pub total: u64,
    pub message: String,
}

fn llama_asset_keywords(hw: &hardware::HardwareInfo) -> Vec<&'static str> {
    match &hw.accelerator {
        Accelerator::AppleSilicon { .. } => vec!["macos", "arm64"],
        Accelerator::Nvidia { .. } => match hw.os.as_str() {
            "windows" => vec!["win", "cuda", "x64"],
            _ => vec!["ubuntu", "x64"],
        },
        Accelerator::Amd { .. } | Accelerator::IntelArc { .. } => match hw.os.as_str() {
            "windows" => vec!["win", "vulkan", "x64"],
            _ => vec!["ubuntu", "vulkan", "x64"],
        },
        Accelerator::Cpu => match hw.os.as_str() {
            "macos" => vec!["macos"],
            "windows" => vec!["win", "cpu", "x64"],
            _ => vec!["ubuntu", "x64"],
        },
    }
}

pub async fn ensure_llama_server(app: &AppHandle) -> Result<PathBuf> {
    let path = config::llama_server_path();
    if path.exists() {
        return Ok(path);
    }

    let hw = hardware::detect();
    let keywords = llama_asset_keywords(&hw);

    emit(
        app,
        "downloading",
        0,
        0,
        &format!("Fetching llama.cpp release info ({})", keywords.join("/")),
    );

    let client = reqwest::Client::builder()
        .user_agent("LocalMind/0.1")
        .build()?;

    let release: serde_json::Value = client
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("no assets in release"))?;

    let asset = pick_llama_asset(assets, &keywords).ok_or_else(|| {
        anyhow!(
            "no matching llama.cpp asset for platform (wanted: {:?})",
            keywords
        )
    })?;

    let url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow!("missing download url"))?;
    let total = asset["size"].as_u64().unwrap_or(0);
    let name = asset["name"]
        .as_str()
        .unwrap_or("llama-archive")
        .to_string();

    emit(
        app,
        "downloading",
        0,
        total,
        &format!("Downloading {}", name),
    );

    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut downloaded: u64 = 0;
    let tmp = config::bin_dir().join(&name);
    let mut file = tokio::fs::File::create(&tmp).await?;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if downloaded % (1024 * 1024) < chunk.len() as u64 {
            emit(
                app,
                "downloading",
                downloaded,
                total,
                "Downloading llama.cpp",
            );
        }
    }
    file.flush().await?;
    drop(file);

    emit(
        app,
        "extracting",
        total,
        total,
        "Extracting llama.cpp binaries",
    );

    let target_dir = config::bin_dir();
    if name.ends_with(".zip") {
        extract_zip(&tmp, &target_dir)?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(&tmp, &target_dir)?;
        flatten_binary(
            &target_dir,
            if cfg!(windows) {
                "llama-server.exe"
            } else {
                "llama-server"
            },
        )?;
    } else {
        return Err(anyhow!("unsupported archive format: {}", name));
    }

    let _ = std::fs::remove_file(&tmp);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.exists() {
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms)?;
        }
        // Ensure shared libs next to the binary are also executable / loadable
        if let Ok(read) = std::fs::read_dir(&target_dir) {
            for e in read.flatten() {
                let p = e.path();
                if p.is_file() {
                    if let Ok(meta) = std::fs::metadata(&p) {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o755);
                        let _ = std::fs::set_permissions(&p, perms);
                    }
                }
            }
        }
    }

    if !path.exists() {
        return Err(anyhow!(
            "llama-server binary not found after extraction at {}",
            path.display()
        ));
    }

    // Windows + CUDA: llama.cpp's CUDA binary ships separately from its CUDA
    // runtime DLLs (`cudart64_*.dll`, `cublas64_*.dll`). Without them next to
    // ggml-cuda.dll, LoadLibrary silently fails and llama.cpp falls back to
    // CPU — the user sees their GPU sitting idle while their CPU pegs at 100%.
    // We pull the matching `cudart-*.zip` from the same release and unpack it
    // alongside.
    if matches!(hw.accelerator, Accelerator::Nvidia { .. }) && hw.os == "windows" {
        // Pull the CUDA version out of the binary archive name (e.g. "13.1"
        // from "llama-b8958-bin-win-cuda-13.1-x64.zip") so we can match the
        // sibling cudart-*.zip exactly. Mismatched versions silently fail at
        // LoadLibrary time.
        let cuda_version = extract_cuda_version(&name);
        if let Err(e) =
            ensure_cuda_runtime(app, &client, assets, &target_dir, cuda_version.as_deref()).await
        {
            // Best-effort — log but don't fail the install. The user gets a
            // working CPU fallback either way; without this hint they'd just
            // wonder why their GPU is idle.
            emit(
                app,
                "warning",
                0,
                0,
                &format!("CUDA runtime fetch failed (worker will use CPU only): {e}"),
            );
        }
    }

    emit(app, "ready", total, total, "llama.cpp ready");
    Ok(path)
}

/// Pull the cudart-* sibling archive from the same llama.cpp release and
/// unzip it next to llama-server.exe / rpc-server.exe. The release exposes
/// archives named like `cudart-llama-bin-win-cu12.4-x64.zip` — pick the one
/// whose CUDA version matches the binary archive we already grabbed.
async fn ensure_cuda_runtime(
    app: &AppHandle,
    client: &reqwest::Client,
    assets: &[serde_json::Value],
    target_dir: &Path,
    cuda_version: Option<&str>,
) -> Result<()> {
    // If cudart64_*.dll already sits next to our binaries, we're done.
    if let Ok(read) = std::fs::read_dir(target_dir) {
        for e in read.flatten() {
            if let Some(name) = e.file_name().to_str() {
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("cudart64_") && lower.ends_with(".dll") {
                    return Ok(());
                }
            }
        }
    }

    // Prefer the cudart whose CUDA version matches the binaries we just
    // downloaded. Fall back to the highest-numbered cudart if we couldn't
    // parse a version. Mismatch (e.g. cu12.4 binaries with cu13.1 cudart)
    // can superficially "load" but produce confusing init failures.
    let cudart_candidates: Vec<&serde_json::Value> = assets
        .iter()
        .filter(|a| {
            let name = a["name"].as_str().unwrap_or("").to_ascii_lowercase();
            name.starts_with("cudart-") && name.contains("win") && name.ends_with(".zip")
        })
        .collect();

    let asset = if let Some(v) = cuda_version {
        cudart_candidates
            .iter()
            .find(|a| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&format!("cuda-{v}"))
            })
            .copied()
    } else {
        None
    }
    .or_else(|| {
        cudart_candidates
            .iter()
            .max_by_key(|a| a["name"].as_str().unwrap_or("").to_string())
            .copied()
    })
    .ok_or_else(|| anyhow!("no cudart-* asset in release"))?;

    let url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow!("missing cudart download url"))?;
    let total = asset["size"].as_u64().unwrap_or(0);
    let name = asset["name"].as_str().unwrap_or("cudart.zip").to_string();

    emit(
        app,
        "downloading",
        0,
        total,
        &format!("Downloading {} (CUDA runtime)", name),
    );

    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut downloaded: u64 = 0;
    let tmp = target_dir.join(&name);
    let mut file = tokio::fs::File::create(&tmp).await?;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if downloaded % (1024 * 1024) < chunk.len() as u64 {
            emit(
                app,
                "downloading",
                downloaded,
                total,
                "Downloading CUDA runtime",
            );
        }
    }
    file.flush().await?;
    drop(file);

    extract_zip(&tmp, target_dir)?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

/// Pull the `MAJOR.MINOR` CUDA version out of a llama.cpp asset name like
/// `llama-b8958-bin-win-cuda-13.1-x64.zip`. Returns `None` if the substring
/// after `cuda-` doesn't look like a version (defensive for future renames).
fn extract_cuda_version(asset_name: &str) -> Option<String> {
    let lower = asset_name.to_ascii_lowercase();
    let after = lower.split("cuda-").nth(1)?;
    // Take chars up to the next non-version separator.
    let v: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if v.is_empty() || !v.contains('.') {
        None
    } else {
        Some(v)
    }
}

fn pick_llama_asset<'a>(
    assets: &'a [serde_json::Value],
    keywords: &[&str],
) -> Option<&'a serde_json::Value> {
    // Prefer archives starting with "llama-" and matching all keywords, avoid "kleidiai" or "cudart-" variants.
    let is_archive = |n: &str| n.ends_with(".zip") || n.ends_with(".tar.gz") || n.ends_with(".tgz");
    let candidates: Vec<&serde_json::Value> = assets
        .iter()
        .filter(|a| {
            let name = a["name"].as_str().unwrap_or("").to_ascii_lowercase();
            is_archive(&name)
                && name.starts_with("llama-")
                && !name.contains("kleidiai")
                && !name.contains("xcframework")
                && keywords.iter().all(|k| name.contains(k))
        })
        .collect();

    // If multiple matches (e.g., cuda-12.4 vs cuda-13.1), prefer the highest version number suffix.
    candidates
        .into_iter()
        .max_by_key(|a| a["name"].as_str().unwrap_or("").to_string())
}

fn flatten_binary(dest: &Path, _target_name: &str) -> Result<()> {
    // Recursively lift every file and symlink from any subdirectory of `dest`
    // up to `dest` itself. Archives may put the binary and its shared libraries
    // in different subdirectories (e.g. build/bin and build/src), so only moving
    // siblings of the primary binary misses required dylibs.
    let subdirs: Vec<PathBuf> = std::fs::read_dir(dest)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();

    for sub in &subdirs {
        lift_all(sub, dest)?;
    }

    // Remove now-empty subdirectories.
    for sub in &subdirs {
        let _ = remove_empty_tree(sub);
    }

    Ok(())
}

fn lift_all(src: &Path, dest: &Path) -> Result<()> {
    let entries: Vec<_> = std::fs::read_dir(src)
        .map(|it| it.flatten().collect())
        .unwrap_or_else(|_| Vec::new());
    for entry in entries {
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            lift_all(&path, dest)?;
            continue;
        }
        let file_name = match path.file_name() {
            Some(n) => n.to_os_string(),
            None => continue,
        };
        let to = dest.join(&file_name);
        if to == path {
            continue;
        }
        if to.exists() || to.is_symlink() {
            // Already present at destination — remove the duplicate in the subdir.
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if std::fs::rename(&path, &to).is_err() {
            // Fallback: copy (handles cross-filesystem) — rename preserves symlinks already.
            if meta.file_type().is_symlink() {
                #[cfg(unix)]
                if let Ok(target) = std::fs::read_link(&path) {
                    let _ = std::os::unix::fs::symlink(&target, &to);
                    let _ = std::fs::remove_file(&path);
                }
            } else {
                let _ = std::fs::copy(&path, &to);
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

fn remove_empty_tree(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries: Vec<_> = std::fs::read_dir(dir)
        .map(|it| it.flatten().collect())
        .unwrap_or_else(|_| Vec::new());
    for entry in entries {
        let p = entry.path();
        if p.is_dir() {
            let _ = remove_empty_tree(&p);
        }
    }
    let _ = std::fs::remove_dir(dir);
    Ok(())
}

pub async fn ensure_sd(app: &AppHandle) -> Result<PathBuf> {
    let path = config::sd_binary_path();
    if path.exists() {
        return Ok(path);
    }

    let hw = hardware::detect();
    let keywords = sd_asset_keywords(&hw);

    emit(
        app,
        "downloading",
        0,
        0,
        "Fetching stable-diffusion.cpp release info",
    );

    let client = reqwest::Client::builder()
        .user_agent("LocalMind/0.1")
        .build()?;

    let release: serde_json::Value = client
        .get("https://api.github.com/repos/leejet/stable-diffusion.cpp/releases/latest")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("no assets in stable-diffusion.cpp release"))?;

    let asset = assets
        .iter()
        .find(|a| {
            let name = a["name"].as_str().unwrap_or("").to_ascii_lowercase();
            (name.ends_with(".zip") || name.ends_with(".tar.gz"))
                && keywords.iter().all(|k| name.contains(k))
        })
        .or_else(|| {
            // Fallback: any archive with the platform keyword
            assets.iter().find(|a| {
                let name = a["name"].as_str().unwrap_or("").to_ascii_lowercase();
                (name.ends_with(".zip") || name.ends_with(".tar.gz")) && name.contains(keywords[0])
            })
        })
        .ok_or_else(|| {
            anyhow!(
                "no matching stable-diffusion.cpp asset for platform (wanted: {:?})",
                keywords
            )
        })?;

    let url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow!("missing download url"))?;
    let total = asset["size"].as_u64().unwrap_or(0);
    let name = asset["name"].as_str().unwrap_or("sd.zip").to_string();

    emit(
        app,
        "downloading",
        0,
        total,
        &format!("Downloading {}", name),
    );

    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut downloaded: u64 = 0;
    let tmp = config::bin_dir().join(&name);
    let mut file = tokio::fs::File::create(&tmp).await?;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if downloaded % (1024 * 1024) < chunk.len() as u64 {
            emit(
                app,
                "downloading",
                downloaded,
                total,
                "Downloading stable-diffusion.cpp",
            );
        }
    }
    file.flush().await?;
    drop(file);

    emit(
        app,
        "extracting",
        total,
        total,
        "Extracting stable-diffusion.cpp binary",
    );

    let target_dir = config::bin_dir();
    if name.ends_with(".zip") {
        extract_zip(&tmp, &target_dir)?;
    } else {
        extract_tar_gz(&tmp, &target_dir)?;
    }

    let _ = std::fs::remove_file(&tmp);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.exists() {
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms)?;
        }
    }

    if !path.exists() {
        return Err(anyhow!(
            "sd binary not found after extraction at {}",
            path.display()
        ));
    }

    emit(app, "ready", total, total, "stable-diffusion.cpp ready");
    Ok(path)
}

fn sd_asset_keywords(hw: &hardware::HardwareInfo) -> Vec<&'static str> {
    match &hw.accelerator {
        Accelerator::AppleSilicon { .. } => vec!["macos", "arm64"],
        Accelerator::Nvidia { .. } => match hw.os.as_str() {
            "windows" => vec!["win", "cuda"],
            _ => vec!["ubuntu", "cuda"],
        },
        Accelerator::Amd { .. } | Accelerator::IntelArc { .. } => match hw.os.as_str() {
            "windows" => vec!["win", "vulkan"],
            _ => vec!["ubuntu", "vulkan"],
        },
        Accelerator::Cpu => match hw.os.as_str() {
            "macos" => vec!["macos"],
            "windows" => vec!["win", "avx2"],
            _ => vec!["ubuntu"],
        },
    }
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    // Minimal tar.gz extraction without extra deps — shell out to system tar
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .context("failed to spawn tar")?;
    if !status.success() {
        return Err(anyhow!("tar extraction failed"));
    }
    let sd_name = if cfg!(windows) { "sd.exe" } else { "sd" };
    flatten_binary(dest, sd_name)?;
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("invalid zip entry"))?
            .to_owned();
        let file_name = name
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        if entry.is_dir() {
            continue;
        }

        let target = dest.join(&file_name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)
            .with_context(|| format!("creating {}", target.display()))?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

fn emit(app: &AppHandle, stage: &str, downloaded: u64, total: u64, message: &str) {
    let _ = app.emit(
        "binary:progress",
        DownloadProgress {
            stage: stage.to_string(),
            downloaded,
            total,
            message: message.to_string(),
        },
    );
}
