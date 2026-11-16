//! Keeps the gameserver registered with the lobby and relays the offers it forwards.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::{Sink, SinkExt, StreamExt};
use protocol::lobby::{GameserverMessage, LobbyMessage};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::{self, Message};
use tracing::{info, warn};

use crate::Config;

/// How often the gameserver reports its player count even when it did not change, which
/// also tells the lobby the connection is alive.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A browser's SDP offer, answered by the game loop with an SDP answer or a rejection reason.
#[derive(Debug)]
pub(crate) struct OfferRequest {
    pub sdp: String,
    pub reply: oneshot::Sender<Result<String, String>>,
}

/// Stays connected to the lobby, reconnecting with exponential backoff.
pub(crate) async fn run(
    config: Arc<Config>,
    offers: mpsc::Sender<OfferRequest>,
    mut players: watch::Receiver<u16>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match session(&config, &offers, &mut players).await {
            Ok(()) => {
                info!("the lobby closed the connection");
                backoff = Duration::from_secs(1);
            }
            Err(error) => warn!("lobby connection failed: {error:#}"),
        }
        if offers.is_closed() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn session(
    config: &Config,
    offers: &mpsc::Sender<OfferRequest>,
    players: &mut watch::Receiver<u16>,
) -> anyhow::Result<()> {
    let mut request = config
        .lobby_url
        .as_str()
        .into_client_request()
        .context("invalid lobby URL")?;
    if let Some(token) = &config.lobby_token {
        request
            .headers_mut()
            .insert(AUTHORIZATION, format!("Bearer {token}").parse()?);
    }
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("connecting to {}", config.lobby_url))?;
    let (mut sink, mut stream) = socket.split();

    send(
        &mut sink,
        GameserverMessage::Register {
            name: config.name.clone(),
            slots: config.slots,
        },
    )
    .await?;
    let count = *players.borrow_and_update();
    send(&mut sink, GameserverMessage::Status { players: count }).await?;

    let (replies_tx, mut replies) = mpsc::channel(16);
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
                message = stream.next() => {
                    let Some(message) = message else { return Ok(()) };
                    match message? {
                        Message::Text(text) => match serde_json::from_str::<LobbyMessage>(text.as_str())? {
                            LobbyMessage::Registered { id } => info!(id, "registered with the lobby"),
                            LobbyMessage::Offer { request_id, sdp } => {
                                let (reply, answer) = oneshot::channel();
                                offers.send(OfferRequest { sdp, reply }).await.context("the game loop stopped")?;
                                let replies = replies_tx.clone();
                                tokio::spawn(async move {
                                    let message = match answer.await {
                                        Ok(Ok(sdp)) => GameserverMessage::Answer { request_id, sdp },
                                        Ok(Err(reason)) => GameserverMessage::Reject { request_id, reason },
                                        Err(_) => GameserverMessage::Reject { request_id, reason: "the gameserver is shutting down".into() },
                                    };
                                    let _ = replies.send(message).await;
                                });
                            }
                        },
                        Message::Close(_) => return Ok(()),
                        _ => {}
                    }
                }
                Some(reply) = replies.recv() => send(&mut sink, reply).await?,
                changed = players.changed() => {
                    changed.context("the game loop stopped")?;
                    let count = *players.borrow_and_update();
        send(&mut sink, GameserverMessage::Status { players: count }).await?;
                }
                _ = heartbeat.tick() => {
                    let count = *players.borrow();
                    send(&mut sink, GameserverMessage::Status { players: count }).await?;
                }
            }
    }
}

async fn send(
    sink: &mut (impl Sink<Message, Error = tungstenite::Error> + Unpin),
    message: GameserverMessage,
) -> anyhow::Result<()> {
    let json = serde_json::to_string(&message)?;
    sink.send(Message::text(json)).await?;
    Ok(())
}
