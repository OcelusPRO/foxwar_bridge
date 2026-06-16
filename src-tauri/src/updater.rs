//! Vérification et installation des mises à jour via les GitHub Releases.
//!
//! Approche volontairement légère (pas d'infrastructure de signature Tauri) :
//! on interroge l'API GitHub pour la dernière release, on compare la version,
//! puis on télécharge et lance l'installeur NSIS qui gère la mise à jour de
//! l'application déjà installée.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};

const REPO: &str = "OcelusPRO/foxwar_bridge";
const USER_AGENT: &str = concat!("foxwar-bridge/", env!("CARGO_PKG_VERSION"));

/// Informations sur une mise à jour disponible, renvoyées au frontend.
#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub download_url: String,
}

/// Frame de progression émise vers le frontend via l'event `update://progress`.
#[derive(Serialize, Clone)]
struct Progress {
    /// "downloading" | "launching"
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
    pct: f64,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Découpe une version `vX.Y.Z` (suffixes pré-release ignorés) en triplet comparable.
fn parse(v: &str) -> (u64, u64, u64) {
    let mut it = v
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .filter_map(|s| s.parse::<u64>().ok());
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

fn is_newer(remote: &str, current: &str) -> bool {
    parse(remote) > parse(current)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())
}

/// Interroge GitHub et renvoie une `UpdateInfo` si une version plus récente que
/// `current` existe. `current` doit être la version embarquée (tauri.conf.json).
pub async fn check(current: &str) -> Result<Option<UpdateInfo>, String> {
    // En build de développement, la version locale (0.1.0) est toujours
    // antérieure à la dernière release publiée : on n'afficherait que de fausses
    // mises à jour. Seuls les builds release (produits par la CI) vérifient.
    if cfg!(debug_assertions) {
        log::info!("Update check skipped (debug build)");
        return Ok(None);
    }

    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let rel: GhRelease = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    log::info!("Update check: current {current}, latest {}", rel.tag_name);
    if !is_newer(&rel.tag_name, current) {
        return Ok(None);
    }

    // On privilégie l'installeur NSIS (-setup.exe), sinon tout .exe, sinon le .msi.
    let asset = rel
        .assets
        .iter()
        .find(|a| a.name.ends_with("-setup.exe"))
        .or_else(|| rel.assets.iter().find(|a| a.name.ends_with(".exe")))
        .or_else(|| rel.assets.iter().find(|a| a.name.ends_with(".msi")))
        .ok_or("Aucun installeur trouvé dans la dernière release")?;

    Ok(Some(UpdateInfo {
        version: rel.tag_name.trim_start_matches('v').to_string(),
        notes: rel.body,
        download_url: asset.browser_download_url.clone(),
    }))
}

/// Télécharge l'installeur (en flux, avec progression émise vers le frontend)
/// puis le lance. L'appelant est responsable de quitter l'application ensuite.
pub async fn download_and_run<R: Runtime>(app: &AppHandle<R>, url: &str) -> Result<(), String> {
    use futures::StreamExt;
    use std::io::Write;

    log::info!("Downloading update from {url}");
    let resp = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Téléchargement impossible : {e}"))?
        .error_for_status()
        .map_err(|e| format!("Réponse HTTP en erreur : {e}"))?;

    let total = resp.content_length();
    let filename = url.rsplit('/').next().unwrap_or("foxwar-bridge-setup.exe");
    let dest = std::env::temp_dir().join(filename);
    let mut file =
        std::fs::File::create(&dest).map_err(|e| format!("Création du fichier impossible : {e}"))?;

    let emit = |phase, downloaded, pct| {
        let _ = app.emit("update://progress", Progress { phase, downloaded, total, pct });
    };

    emit("downloading", 0, 0.0);

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_pct = -1.0_f64;
    let mut last_emit: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Lecture du flux impossible : {e}"))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Écriture du fichier impossible : {e}"))?;
        downloaded += chunk.len() as u64;

        let pct = match total {
            Some(t) if t > 0 => downloaded as f64 / t as f64 * 100.0,
            _ => 0.0,
        };
        // Limite la fréquence des events : tous les 1 % (taille connue) ou 256 Ko.
        let should_emit = match total {
            Some(_) => pct - last_pct >= 1.0,
            None => downloaded - last_emit >= 262_144,
        };
        if should_emit {
            last_pct = pct;
            last_emit = downloaded;
            emit("downloading", downloaded, pct);
        }
    }
    file.flush().ok();
    drop(file);

    log::info!("Update downloaded to {} ({downloaded} bytes)", dest.display());
    emit("launching", downloaded, 100.0);

    std::process::Command::new(&dest)
        .spawn()
        .map_err(|e| format!("Impossible de lancer l'installeur : {e}"))?;

    log::info!("Installer launched, application will exit");
    Ok(())
}
