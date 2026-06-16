use std::sync::Arc;

use tauri::Manager;
use url::Url;

use crate::{AppState, SseServerHandle};

/// Traite les arguments de ligne de commande passés à l'instance principale.
pub fn handle_argv(app: &tauri::AppHandle, argv: &[String]) {
    for arg in argv {
        if arg.starts_with("foxwar://") {
            handle_url(app, arg);
            return;
        }
    }
    bring_window_front(app);
}

/// Dispatch d'une URL `foxwar://`.
pub fn handle_url(app: &tauri::AppHandle, raw: &str) {
    let url = match Url::parse(raw) {
        Ok(u) => u,
        Err(e) => {
            log::warn!("Malformed foxwar:// URL '{raw}': {e}");
            bring_window_front(app);
            return;
        }
    };

    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();

    match url.host_str().unwrap_or("open") {
        // ── foxwar://connect?port=7842&token=XXX&origin=https://… ─────────────
        // Lance (ou relance) le serveur SSE sur le port demandé.
        "connect" => {
            let port = params
                .get("port")
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(7842);
            let token = params
                .get("token")
                .map(|t| t.to_string())
                .unwrap_or_else(crate::settings::generate_token);
            let origin = params
                .get("origin")
                .map(|o| o.to_string())
                .unwrap_or_else(|| "*".to_string());

            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                start_sse_server(app, port, token, origin).await;
            });
        }

        // ── foxwar://configure?origin=https://… ──────────────────────────────
        // Met à jour l'origine CORS autorisée.
        "configure" => {
            if let Some(origin) = params.get("origin") {
                let state = app.state::<AppState>();
                let mut s = state.settings.write().unwrap();
                s.allowed_origin = origin.to_string();
                let _ = s.save();
                log::info!("Allowed origin → {origin}");
            }
            bring_window_front(app);
        }

        // ── foxwar://refresh ──────────────────────────────────────────────────
        // Force la relecture immédiate du fichier SAV.
        "refresh" => {
            let state = app.state::<AppState>();
            let settings = state.settings.read().unwrap();
            if let Some(ref dir) = settings.sav_path {
                let dir = std::path::PathBuf::from(dir);
                let tx = state.sse_tx.clone();
                let last = std::sync::Arc::clone(&state.last_event);
                drop(settings);
                crate::watcher::broadcast_latest_in_dir(&dir, &tx, &last);
            }
        }

        // ── foxwar://open (ou autre) → afficher la fenêtre ───────────────────
        _ => bring_window_front(app),
    }
}

// ─── Démarrage du serveur SSE ─────────────────────────────────────────────────

/// Démarre (ou redémarre) le serveur SSE axum sur `port` avec `token`.
/// Arrête l'éventuel serveur précédent avant.
async fn start_sse_server(app: tauri::AppHandle, port: u16, token: String, origin: String) {
    let state = app.state::<AppState>();

    // Arrêter le serveur précédent (en droppant son SseServerHandle)
    *state.sse_server.lock().unwrap() = None;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let handle = SseServerHandle {
        port,
        token: token.clone(),
        _shutdown: shutdown_tx,
    };
    *state.sse_server.lock().unwrap() = Some(handle);

    // Persister le token et l'origine pour l'UI
    {
        let mut s = state.settings.write().unwrap();
        s.token = token.clone();
        if origin != "*" { s.allowed_origin = origin.clone(); }
        let _ = s.save();
    }

    let tx = state.sse_tx.clone();
    let cc = Arc::clone(&state.client_count);
    let settings = Arc::clone(&state.settings);

    log::info!("Starting SSE server on port {port}");
    crate::sse_server::start(tx, token, port, origin, cc, settings, shutdown_rx).await;
    log::info!("SSE server on port {port} stopped");
}

fn bring_window_front(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        win.show().ok();
        win.set_focus().ok();
    }
}
