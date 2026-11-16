//! The lobby lists gameservers and relays WebRTC signaling between browsers and gameservers.
//!
//! * Gameservers keep a WebSocket open to [`GAMESERVER_WS_PATH`]: they register, report their
//!   player count, and answer the offers relayed to them.
//! * Browsers fetch `GET /api/servers`, then `POST /api/servers/{id}/connect` with their SDP
//!   offer and receive the gameserver's answer in the response. After that, browser and
//!   gameserver talk directly over WebRTC and the lobby is out of the loop.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use protocol::lobby::{
    ApiError, GAMESERVER_WS_PATH, GameserverMessage, LobbyMessage, MAX_SDP_LEN,
    MAX_SERVER_NAME_LEN, ServerSummary, SessionDescription,
};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

/// How long a gameserver has to register after connecting.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the lobby pings gameservers; three silent intervals drop the connection.
const PING_INTERVAL: Duration = Duration::from_secs(10);
/// Most players a gameserver may advertise.
const MAX_SLOTS: u16 = 64;

#[derive(Debug, Clone)]
pub struct Config {
    /// Secret gameservers must send as `Authorization: Bearer <token>`. With `None`, anyone may
    /// register a gameserver, which is only acceptable during development.
    pub gameserver_token: Option<String>,
    /// How long a browser waits for the gameserver's answer.
    pub answer_timeout: Duration,
    /// The built client, served on every path outside the API.
    pub static_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gameserver_token: None,
            answer_timeout: Duration::from_secs(5),
            static_dir: None,
        }
    }
}

#[derive(Debug, Clone)]
struct AppState(Arc<Lobby>);

#[derive(Debug)]
struct Lobby {
    config: Config,
    servers: Mutex<BTreeMap<String, Gameserver>>,
    next_server_id: AtomicU64,
    next_request_id: AtomicU64,
}

#[derive(Debug)]
struct Gameserver {
    summary: ServerSummary,
    outbox: mpsc::Sender<LobbyMessage>,
    /// Browsers waiting for an answer; dropped (failing their requests) when the gameserver leaves.
    pending: HashMap<u64, oneshot::Sender<Result<String, String>>>,
}

impl AppState {
    fn servers(&self) -> MutexGuard<'_, BTreeMap<String, Gameserver>> {
        self.0
            .servers
            .lock()
            .expect("the server registry lock is never poisoned")
    }
}

/// Builds the lobby's HTTP application.
pub fn router(config: Config) -> Router {
    let static_dir = config.static_dir.clone();
    let state = AppState(Arc::new(Lobby {
        config,
        servers: Mutex::default(),
        next_server_id: AtomicU64::new(1),
        next_request_id: AtomicU64::new(1),
    }));
    let api = Router::new()
        .route("/api/servers", get(list_servers))
        .route("/api/servers/{id}/connect", post(connect))
        .route(GAMESERVER_WS_PATH, get(gameserver_socket))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(MAX_SDP_LEN + 1024))
        .with_state(state);
    let app = match static_dir {
        Some(dir) => {
            let index = ServeFile::new(dir.join("index.html"));
            api.fallback_service(ServeDir::new(dir).fallback(index))
        }
        None => api,
    };
    app.layer(TraceLayer::new_for_http())
}

/// Serves the lobby on `listener` until `shutdown` completes.
///
/// # Errors
/// Fails when accepting connections fails.
pub async fn serve(
    listener: TcpListener,
    config: Config,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(config))
        .with_graceful_shutdown(shutdown)
        .await
}

#[derive(Debug)]
struct HttpError(StatusCode, String);

impl HttpError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self(status, message.into())
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.0, Json(ApiError { error: self.1 })).into_response()
    }
}

async fn list_servers(State(state): State<AppState>) -> Json<Vec<ServerSummary>> {
    let mut servers: Vec<ServerSummary> = state
        .servers()
        .values()
        .map(|server| server.summary.clone())
        .collect();
    servers.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    Json(servers)
}

async fn connect(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(offer): Json<SessionDescription>,
) -> Result<Json<SessionDescription>, HttpError> {
    if offer.sdp.len() > MAX_SDP_LEN {
        return Err(HttpError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "the offer is too large",
        ));
    }
    let request_id = state.0.next_request_id.fetch_add(1, Ordering::Relaxed);
    let (answer_tx, answer_rx) = oneshot::channel();
    let outbox = {
        let mut servers = state.servers();
        let Some(server) = servers.get_mut(&id) else {
            return Err(HttpError::new(StatusCode::NOT_FOUND, "no such server"));
        };
        if server.summary.players >= server.summary.slots {
            return Err(HttpError::new(StatusCode::CONFLICT, "the server is full"));
        }
        server.pending.insert(request_id, answer_tx);
        server.outbox.clone()
    };

    let gone = || HttpError::new(StatusCode::BAD_GATEWAY, "the server went away");
    outbox
        .send(LobbyMessage::Offer {
            request_id,
            sdp: offer.sdp,
        })
        .await
        .map_err(|_| gone())?;
    match tokio::time::timeout(state.0.config.answer_timeout, answer_rx).await {
        Ok(Ok(Ok(sdp))) => Ok(Json(SessionDescription { sdp })),
        Ok(Ok(Err(reason))) => Err(HttpError::new(StatusCode::CONFLICT, reason)),
        Ok(Err(_)) => Err(gone()),
        Err(_) => {
            if let Some(server) = state.servers().get_mut(&id) {
                server.pending.remove(&request_id);
            }
            Err(HttpError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "the server did not answer in time",
            ))
        }
    }
}

async fn gameserver_socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Some(token) = &state.0.config.gameserver_token {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if !presented
            .is_some_and(|presented| constant_time_eq(presented.as_bytes(), token.as_bytes()))
        {
            return HttpError::new(StatusCode::UNAUTHORIZED, "invalid gameserver token")
                .into_response();
        }
    }
    upgrade
        .max_message_size(MAX_SDP_LEN * 2)
        .on_upgrade(move |socket| handle_gameserver(state, socket))
}

async fn handle_gameserver(state: AppState, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();

    let registration = tokio::time::timeout(REGISTER_TIMEOUT, async {
        while let Some(Ok(message)) = stream.next().await {
            if let Message::Text(text) = message {
                return serde_json::from_str::<GameserverMessage>(text.as_str()).ok();
            }
        }
        None
    })
    .await;
    let Ok(Some(GameserverMessage::Register { name, slots })) = registration else {
        warn!("a gameserver connected without registering");
        let _ = sink.close().await;
        return;
    };

    let id = state
        .0
        .next_server_id
        .fetch_add(1, Ordering::Relaxed)
        .to_string();
    let summary = ServerSummary {
        id: id.clone(),
        name: sanitize_name(&name),
        players: 0,
        slots: slots.clamp(1, MAX_SLOTS),
    };
    info!(
        id,
        name = summary.name,
        slots = summary.slots,
        "gameserver registered"
    );
    let (outbox, mut inbox) = mpsc::channel(32);
    state.servers().insert(
        id.clone(),
        Gameserver {
            summary,
            outbox,
            pending: HashMap::new(),
        },
    );

    if send(&mut sink, &LobbyMessage::Registered { id: id.clone() })
        .await
        .is_ok()
    {
        relay(&state, &id, &mut sink, &mut stream, &mut inbox).await;
    }

    state.servers().remove(&id);
    info!(id, "gameserver left");
}

/// Shuttles messages between a registered gameserver and the browsers talking to it.
async fn relay(
    state: &AppState,
    id: &str,
    sink: &mut SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    inbox: &mut mpsc::Receiver<LobbyMessage>,
) {
    let mut ping = tokio::time::interval(PING_INTERVAL);
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            message = stream.next() => {
                let Some(Ok(message)) = message else { return };
                last_seen = Instant::now();
                match message {
                    Message::Text(text) => match serde_json::from_str::<GameserverMessage>(text.as_str()) {
                        Ok(message) => handle_gameserver_message(state, id, message),
                        Err(error) => warn!(id, %error, "invalid message from gameserver"),
                    },
                    Message::Close(_) => return,
                    Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {}
                }
            }
            Some(message) = inbox.recv() => {
                if send(sink, &message).await.is_err() {
                    return;
                }
            }
            _ = ping.tick() => {
                if last_seen.elapsed() > PING_INTERVAL * 3 {
                    warn!(id, "gameserver stopped responding");
                    return;
                }
                if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn handle_gameserver_message(state: &AppState, id: &str, message: GameserverMessage) {
    let mut servers = state.servers();
    let Some(server) = servers.get_mut(id) else {
        return;
    };
    match message {
        GameserverMessage::Status { players } => server.summary.players = players,
        GameserverMessage::Answer { request_id, sdp } => {
            if let Some(waiting) = server.pending.remove(&request_id) {
                let _ = waiting.send(Ok(sdp));
            }
        }
        GameserverMessage::Reject { request_id, reason } => {
            if let Some(waiting) = server.pending.remove(&request_id) {
                let _ = waiting.send(Err(reason));
            }
        }
        GameserverMessage::Register { .. } => warn!(id, "gameserver registered twice"),
    }
}

async fn send(
    sink: &mut SplitSink<WebSocket, Message>,
    message: &LobbyMessage,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(message).expect("lobby messages always serialize");
    sink.send(Message::Text(json.into())).await
}

fn sanitize_name(name: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_SERVER_NAME_LEN)
        .collect();
    if name.is_empty() {
        "Unnamed server".to_owned()
    } else {
        name
    }
}

/// Compares secrets without leaking, through timing, how long a matching prefix is.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_truncated() {
        assert_eq!(sanitize_name("  Dust\u{7}  "), "Dust");
        assert_eq!(sanitize_name(""), "Unnamed server");
        assert_eq!(sanitize_name(&"x".repeat(100)).len(), MAX_SERVER_NAME_LEN);
    }

    #[test]
    fn secrets_compare_exactly() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret!"));
    }
}
