use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use chrono::{DateTime, Utc};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use regex::Regex;
use tauri::Manager;
use tokio::sync::broadcast;

pub type WatcherHandle = Option<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>>;

/// Regex correspondant aux noms de fichiers MapData.sav de Foxhole.
fn sav_pattern() -> Regex {
    Regex::new(r"^\d{15,20}_MapData\.sav$").expect("Invalid SAV regex")
}

/// Démarre la surveillance du répertoire `dir`.
/// Remplace tout watcher existant dans l'état de l'app.
pub fn start(app: &tauri::AppHandle, dir: impl AsRef<Path>) {
    use crate::AppState;

    let dir = dir.as_ref().to_path_buf();
    if !dir.exists() {
        log::warn!("Watch dir does not exist yet: {}", dir.display());
    }

    // Clone les champs nécessaires depuis l'état partagé
    let (sse_tx, last_event, watcher_arc) = {
        let state = app.state::<AppState>();
        (
            state.sse_tx.clone(),
            Arc::clone(&state.last_event),
            Arc::clone(&state.watcher),
        )
    };

    let pattern = sav_pattern();
    let (sync_tx, sync_rx) = std::sync::mpsc::channel();

    let mut debouncer = match new_debouncer(std::time::Duration::from_millis(500), sync_tx) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create file watcher: {e}");
            return;
        }
    };

    if let Err(e) = debouncer.watcher().watch(&dir, RecursiveMode::NonRecursive) {
        log::error!("Failed to watch {}: {e}", dir.display());
        return;
    }

    // Thread dédié : pont sync mpsc → broadcast SSE
    let sse_tx_clone = sse_tx.clone();
    let last_event_clone = Arc::clone(&last_event);
    std::thread::spawn(move || {
        for result in sync_rx.iter() {
            match result {
                Ok(events) => {
                    for ev in events {
                        if ev.kind != DebouncedEventKind::Any { continue; }
                        let name = ev.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if pattern.is_match(name) {
                            broadcast_file(&ev.path, &sse_tx_clone, &last_event_clone);
                        }
                    }
                }
                Err(e) => log::error!("Watch error: {e}"),
            }
        }
    });

    *watcher_arc.lock().unwrap() = Some(debouncer);
    log::info!("Watching: {}", dir.display());
}

/// Arrête le watcher courant (libère la ressource, le thread se termine naturellement).
pub fn stop(watcher_arc: &Arc<Mutex<WatcherHandle>>) {
    *watcher_arc.lock().unwrap() = None;
}

/// Relit le fichier SAV le plus récent dans `dir` et le diffuse via SSE.
pub fn broadcast_latest_in_dir(
    dir: &Path,
    tx: &broadcast::Sender<String>,
    last_event: &Arc<Mutex<Option<DateTime<Utc>>>>,
) {
    match latest_sav(dir) {
        Some(path) => broadcast_file(&path, tx, last_event),
        None => log::debug!("No MapData.sav found in {}", dir.display()),
    }
}

/// Construit le payload SSE JSON pour le fichier SAV le plus récent de `dir`,
/// sans le diffuser. Utilisé pour pousser l'état courant à un client qui vient
/// de se connecter.
pub fn build_latest_payload(dir: &Path) -> Option<String> {
    read_payload(&latest_sav(dir)?)
}

/// Trouve le fichier MapData.sav le plus récent (par date de modification) dans `dir`.
fn latest_sav(dir: &Path) -> Option<PathBuf> {
    let pattern = sav_pattern();
    std::fs::read_dir(dir).ok().and_then(|entries| {
        entries
            .filter_map(|e| e.ok())
            .filter(|e| pattern.is_match(e.file_name().to_str().unwrap_or("")))
            .map(|e| e.path())
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
    })
}

/// Lit `path`, encode en base64 et construit le payload JSON `sav_updated`.
fn read_payload(path: &Path) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let payload = serde_json::json!({
                "type": "sav_updated",
                "timestamp": Utc::now().timestamp(),
                "path": path.to_string_lossy(),
                "file": b64
            })
            .to_string();
            log::info!("Read SAV: {} ({} bytes)", path.display(), bytes.len());
            Some(payload)
        }
        Err(e) => {
            log::error!("Failed to read {}: {e}", path.display());
            None
        }
    }
}

/// Construit le payload du fichier et le diffuse à tous les abonnés SSE.
fn broadcast_file(
    path: &Path,
    tx: &broadcast::Sender<String>,
    last_event: &Arc<Mutex<Option<DateTime<Utc>>>>,
) {
    if let Some(payload) = read_payload(path) {
        if tx.send(payload).is_err() {
            log::debug!("No SSE subscribers at the moment");
        }
        *last_event.lock().unwrap() = Some(Utc::now());
    }
}
