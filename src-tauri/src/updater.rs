//! Vérification et installation des mises à jour via les GitHub Releases.
//!
//! Approche volontairement légère (pas d'infrastructure de signature Tauri) :
//! on interroge l'API GitHub pour la dernière release, on compare la version,
//! puis on télécharge et lance l'installeur NSIS qui gère la mise à jour de
//! l'application déjà installée.

use serde::{Deserialize, Serialize};

const REPO: &str = "OcelusPRO/foxwar_bridge";
const USER_AGENT: &str = concat!("foxwar-bridge/", env!("CARGO_PKG_VERSION"));

/// Informations sur une mise à jour disponible, renvoyées au frontend.
#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub download_url: String,
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
        .build()
        .map_err(|e| e.to_string())
}

/// Interroge GitHub et renvoie une `UpdateInfo` si une version plus récente existe.
pub async fn check() -> Result<Option<UpdateInfo>, String> {
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

    if !is_newer(&rel.tag_name, env!("CARGO_PKG_VERSION")) {
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

/// Télécharge l'installeur dans le dossier temp et le lance.
/// L'appelant est responsable de quitter l'application ensuite.
pub async fn download_and_run(url: &str) -> Result<(), String> {
    let bytes = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;

    let filename = url.rsplit('/').next().unwrap_or("foxwar-bridge-setup.exe");
    let dest = std::env::temp_dir().join(filename);
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;

    log::info!("Update downloaded to {} ({} bytes)", dest.display(), bytes.len());

    std::process::Command::new(&dest)
        .spawn()
        .map_err(|e| format!("Impossible de lancer l'installeur : {e}"))?;

    Ok(())
}
