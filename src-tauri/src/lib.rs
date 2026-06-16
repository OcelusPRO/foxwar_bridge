pub mod commands;
pub mod protocol;
pub mod settings;
pub mod sse_server;
pub mod tray;
pub mod watcher;

use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::AtomicUsize;

use chrono::{DateTime, Utc};
use settings::Settings;
use tauri::Manager;
use tokio::sync::broadcast;

// ─── Handle du serveur SSE ────────────────────────────────────────────────────

/// Maintient le serveur SSE actif tant qu'il est en vie.
/// Dropper ce struct envoie le signal d'arrêt au serveur axum.
pub struct SseServerHandle {
    pub port: u16,
    pub token: String,
    /// Dropping this sender signals the server to shut down.
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

// ─── État partagé ─────────────────────────────────────────────────────────────

pub struct AppState {
    pub settings: Arc<RwLock<Settings>>,
    pub sse_tx: broadcast::Sender<String>,
    /// Watcher de fichier — None = surveillance inactive.
    pub watcher: Arc<Mutex<watcher::WatcherHandle>>,
    /// Serveur SSE — None = serveur éteint (état par défaut au démarrage).
    pub sse_server: Arc<Mutex<Option<SseServerHandle>>>,
    pub client_count: Arc<AtomicUsize>,
    pub last_event: Arc<Mutex<Option<DateTime<Utc>>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let settings = Settings::load();
    if let Err(e) = settings.save() {
        log::warn!("Could not persist initial settings: {e}");
    }

    let (sse_tx, _) = broadcast::channel::<String>(256);
    let client_count = Arc::new(AtomicUsize::new(0));

    let state = AppState {
        settings: Arc::new(RwLock::new(settings)),
        sse_tx,
        watcher: Arc::new(Mutex::new(None)),
        // Le serveur SSE démarre DÉSACTIVÉ — activé uniquement via foxwar://connect
        sse_server: Arc::new(Mutex::new(None)),
        client_count,
        last_event: Arc::new(Mutex::new(None)),
    };

    tauri::Builder::default()
        // ── Plugins ──────────────────────────────────────────────────────────
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            log::info!("Second instance forwarding: {argv:?}");
            protocol::handle_argv(app, &argv);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        // ── État ─────────────────────────────────────────────────────────────
        .manage(state)
        // ── Setup ────────────────────────────────────────────────────────────
        .setup(|app| {
            // Deep-link reçu quand l'app est déjà ouverte
            let handle = app.handle().clone();
            app.listen("deep-link://new-url", move |event| {
                if let Ok(urls) = serde_json::from_str::<Vec<String>>(event.payload()) {
                    for url in urls {
                        protocol::handle_url(&handle, &url);
                    }
                }
            });

            tray::setup(app)?;

            // Démarrage du watcher de fichier (surveille en arrière-plan, SSE off)
            let sav_path = app.state::<AppState>().settings.read().unwrap().sav_path.clone();
            if let Some(dir) = sav_path {
                watcher::start(app.handle(), &dir);
            }

            // URLs foxwar:// passées à ce lancement (1re instance)
            for arg in std::env::args().skip(1) {
                if arg.starts_with("foxwar://") {
                    protocol::handle_url(app.handle(), &arg);
                }
            }

            Ok(())
        })
        // ── Fermeture fenêtre → tray ──────────────────────────────────────────
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap_or_default();
                api.prevent_close();
            }
        })
        // ── Commandes ─────────────────────────────────────────────────────────
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_status,
            commands::set_autostart,
            commands::set_sav_path,
            commands::set_allowed_origin,
            commands::regenerate_token,
            commands::pick_directory,
            commands::trigger_refresh,
            commands::get_sse_status,
        ])
        .run(tauri::generate_context!())
        .expect("Foxwar Bridge crashed");
}
