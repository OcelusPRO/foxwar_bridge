use std::path::Path;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{command, AppHandle, State};
use tauri_plugin_autostart::{AutoLaunchManager, ManagerExt};

use crate::{settings::generate_token, AppState};

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct BridgeStatus {
    pub watcher_active: bool,
    pub sav_path_exists: bool,
    pub client_count: usize,
    pub last_event: Option<DateTime<Utc>>,
    pub autostart: bool,
    pub port: u16,
}

#[derive(Serialize)]
pub struct SseStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub token: Option<String>,
    pub client_count: usize,
}

// ─── Commandes ───────────────────────────────────────────────────────────────

#[command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<crate::settings::Settings, String> {
    Ok(state.settings.read().unwrap().clone())
}

#[command]
pub async fn get_status<R: tauri::Runtime>(state: State<'_, AppState>, app: AppHandle<R>) -> Result<BridgeStatus, String> {
    let settings = state.settings.read().unwrap();
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    Ok(BridgeStatus {
        watcher_active: state.watcher.lock().unwrap().is_some(),
        sav_path_exists: settings.sav_path.as_deref().map(|p| Path::new(p).exists()).unwrap_or(false),
        client_count: state.client_count.load(Ordering::Relaxed),
        last_event: *state.last_event.lock().unwrap(),
        autostart,
        port: settings.port,
    })
}

/// État du serveur SSE (actif ou non, port et token courants).
#[command]
pub async fn get_sse_status(state: State<'_, AppState>) -> Result<SseStatus, String> {
    let lock = state.sse_server.lock().unwrap();
    Ok(match lock.as_ref() {
        Some(h) => SseStatus {
            running: true,
            port: Some(h.port),
            token: Some(h.token.clone()),
            client_count: state.client_count.load(Ordering::Relaxed),
        },
        None => SseStatus {
            running: false,
            port: None,
            token: None,
            client_count: 0,
        },
    })
}

#[command]
pub async fn set_autostart<R: tauri::Runtime>(enabled: bool, app: AppHandle<R>, state: State<'_, AppState>) -> Result<(), String> {
    let manager: State<'_, AutoLaunchManager> = app.autolaunch();
    if enabled { manager.enable().map_err(|e| e.to_string())?; }
    else        { manager.disable().map_err(|e| e.to_string())?; }
    state.settings.write().unwrap().autostart = enabled;
    state.settings.read().unwrap().save().map_err(|e| e.to_string())?;
    Ok(())
}

#[command]
pub async fn set_sav_path(path: String, state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    let changed = state.settings.read().unwrap().sav_path.as_deref() != Some(&path);
    state.settings.write().unwrap().sav_path = Some(path.clone());
    state.settings.read().unwrap().save().map_err(|e| e.to_string())?;
    if changed {
        crate::watcher::stop(&state.watcher);
        crate::watcher::start(&app, &path);
    }
    Ok(())
}

#[command]
pub async fn set_allowed_origin(origin: String, state: State<'_, AppState>) -> Result<(), String> {
    state.settings.write().unwrap().allowed_origin = origin;
    state.settings.read().unwrap().save().map_err(|e| e.to_string())
}

#[command]
pub async fn regenerate_token(state: State<'_, AppState>) -> Result<String, String> {
    let token = generate_token();
    state.settings.write().unwrap().token = token.clone();
    state.settings.read().unwrap().save().map_err(|e| e.to_string())?;
    Ok(token)
}

#[command]
pub async fn pick_directory(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let result = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Sélectionner le dossier SaveGames de Foxhole").blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.map(|fp| {
        let buf: std::path::PathBuf = fp.into_path().unwrap_or_default();
        buf.to_string_lossy().into_owned()
    }))
}

#[command]
pub async fn trigger_refresh(state: State<'_, AppState>) -> Result<(), String> {
    let (dir, tx, last) = {
        let s = state.settings.read().unwrap();
        (s.sav_path.clone().map(std::path::PathBuf::from), state.sse_tx.clone(), std::sync::Arc::clone(&state.last_event))
    };
    match dir {
        Some(d) => { crate::watcher::broadcast_latest_in_dir(&d, &tx, &last); Ok(()) }
        None => Err("Aucun chemin SAV configuré".to_string()),
    }
}

// ─── Mises à jour ──────────────────────────────────────────────────────────────

/// Version courante de l'application, lue depuis le bundle (tauri.conf.json).
/// C'est la même source que le nom de l'installeur → toujours cohérente.
#[command]
pub fn get_version<R: tauri::Runtime>(app: AppHandle<R>) -> String {
    app.package_info().version.to_string()
}

/// Vérifie si une mise à jour est disponible sur GitHub Releases.
#[command]
pub async fn check_update<R: tauri::Runtime>(app: AppHandle<R>) -> Result<Option<crate::updater::UpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    crate::updater::check(&current).await
}

/// Télécharge et lance l'installeur de la mise à jour, puis quitte l'app.
/// La progression est émise via l'event `update://progress`.
#[command]
pub async fn install_update<R: tauri::Runtime>(url: String, app: AppHandle<R>) -> Result<(), String> {
    crate::updater::download_and_run(&app, &url).await?;
    // Laisse l'installeur démarrer avant de libérer l'exe en cours d'exécution.
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        app2.exit(0);
    });
    Ok(())
}
