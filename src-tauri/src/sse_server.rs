use std::path::Path;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{collections::HashMap, convert::Infallible};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use futures::StreamExt;
use serde::Serialize;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::settings::Settings;

#[derive(Clone)]
pub struct SseState {
    pub token: String,
    pub tx: broadcast::Sender<String>,
    pub client_count: Arc<AtomicUsize>,
    /// Settings partagés : permet de lire `sav_path` à la connexion d'un client.
    pub settings: Arc<RwLock<Settings>>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    clients: usize,
}

/// Lance le serveur axum sur `127.0.0.1:port`.
/// S'arrête dès que `shutdown` reçoit un signal (ou que l'émetteur est droppé).
pub async fn start(
    tx: broadcast::Sender<String>,
    token: String,
    port: u16,
    allowed_origin: String,
    client_count: Arc<AtomicUsize>,
    settings: Arc<RwLock<Settings>>,
    shutdown: oneshot::Receiver<()>,
) {
    let state = SseState { token, tx, client_count, settings };
    let cors = build_cors(&allowed_origin);

    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/health", get(health_handler))
        .layer(cors)
        .with_state(state);

    let addr = format!("127.0.0.1:{port}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("Cannot bind SSE server on {addr}: {e}");
            return;
        }
    };

    log::info!("SSE server listening on http://{addr}");

    tokio::select! {
        result = axum::serve(listener, app) => {
            if let Err(e) = result { log::error!("SSE server error: {e}"); }
        }
        _ = async { let _ = shutdown.await; } => {
            log::info!("SSE server on {addr} received shutdown signal");
        }
    }
}

// ─── Drop guard ───────────────────────────────────────────────────────────────

struct ClientGuard(Arc<AtomicUsize>);
impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

// ─── Handlers ────────────────────────────────────────────────────────────────

async fn sse_handler(
    Query(params): Query<HashMap<String, String>>,
    State(state): State<SseState>,
) -> Result<
    Sse<impl futures::Stream<Item = Result<Event, Infallible>>>,
    (StatusCode, &'static str),
> {
    let token = params.get("token").map(|s| s.as_str()).unwrap_or("");
    if token != state.token {
        return Err((StatusCode::UNAUTHORIZED, "Invalid or missing token"));
    }

    state.client_count.fetch_add(1, Ordering::Relaxed);
    let guard = ClientGuard(Arc::clone(&state.client_count));

    // ── Envoi immédiat de l'état courant ──────────────────────────────────────
    // Le nouveau client reçoit directement le dernier MapData.sav connu, sans
    // attendre la prochaine modification de fichier.
    let dir_opt = state.settings.read().unwrap().sav_path.clone();
    let initial = match dir_opt {
        Some(dir) => tokio::task::spawn_blocking(move || {
            crate::watcher::build_latest_payload(Path::new(&dir))
        })
        .await
        .ok()
        .flatten(),
        None => None,
    };
    let initial_stream = futures::stream::iter(
        initial
            .into_iter()
            .map(|data| Ok::<Event, Infallible>(Event::default().data(data))),
    );

    // ── Flux live des modifications suivantes ─────────────────────────────────
    let rx = state.tx.subscribe();
    let live = BroadcastStream::new(rx)
        .filter_map(|r| async { r.ok() })
        .map(move |data| {
            let _ = &guard;
            Ok::<Event, Infallible>(Event::default().data(data))
        });

    let stream = initial_stream.chain(live);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn health_handler(State(state): State<SseState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        clients: state.client_count.load(Ordering::Relaxed),
    })
}

// ─── CORS ────────────────────────────────────────────────────────────────────

fn build_cors(allowed_origin: &str) -> CorsLayer {
    let origin = if allowed_origin == "*" {
        AllowOrigin::any()
    } else {
        match allowed_origin.parse::<axum::http::HeaderValue>() {
            Ok(hv) => AllowOrigin::exact(hv),
            Err(_) => {
                log::warn!("Invalid CORS origin '{allowed_origin}', falling back to *");
                AllowOrigin::any()
            }
        }
    };
    CorsLayer::new()
        .allow_origin(origin)
        .allow_methods([axum::http::Method::GET])
}
