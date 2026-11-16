//! JSON messages of the lobby.
//!
//! Browsers talk to the lobby over HTTP:
//! * `GET /api/servers` returns `Vec<ServerSummary>`.
//! * `POST /api/servers/{id}/connect` takes a [`SessionDescription`] holding the browser's SDP
//!   offer, relays it to the gameserver and responds with the gameserver's SDP answer.
//!
//! Gameservers keep a WebSocket open to [`GAMESERVER_WS_PATH`], exchanging
//! [`GameserverMessage`] and [`LobbyMessage`] as JSON text frames. Gameservers dial out, so
//! they need no public HTTP endpoint or TLS certificate, only a reachable UDP port.

use serde::{Deserialize, Serialize};

/// Path of the lobby's WebSocket endpoint for gameservers.
pub const GAMESERVER_WS_PATH: &str = "/api/gameservers";

/// Longest server name the lobby accepts; longer names are truncated.
pub const MAX_SERVER_NAME_LEN: usize = 32;

/// Largest SDP (in bytes) the lobby relays.
pub const MAX_SDP_LEN: usize = 16 * 1024;

/// A gameserver as listed by `GET /api/servers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
pub struct ServerSummary {
    pub id: String,
    pub name: String,
    pub players: u16,
    pub slots: u16,
}

/// An SDP offer (request body of `POST /api/servers/{id}/connect`) or answer (its response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
pub struct SessionDescription {
    pub sdp: String,
}

/// Body of every non-2xx response of the lobby HTTP API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
pub struct ApiError {
    pub error: String,
}

/// Messages a gameserver sends to the lobby.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GameserverMessage {
    /// Must be the first message after connecting.
    Register { name: String, slots: u16 },
    /// Sent whenever the number of connected players changes.
    Status { players: u16 },
    /// The answer to [`LobbyMessage::Offer`].
    Answer { request_id: u64, sdp: String },
    /// Sent instead of an answer when the offer cannot be accepted.
    Reject { request_id: u64, reason: String },
}

/// Messages the lobby sends to a gameserver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LobbyMessage {
    /// Confirms [`GameserverMessage::Register`] with the id clients use to connect.
    Registered { id: String },
    /// A browser's SDP offer; reply with [`GameserverMessage::Answer`] or [`GameserverMessage::Reject`].
    Offer { request_id: u64, sdp: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameserver_messages_use_tagged_camel_case_json() {
        let msg = GameserverMessage::Answer {
            request_id: 7,
            sdp: "v=0".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"answer","requestId":7,"sdp":"v=0"}"#);
        assert_eq!(
            serde_json::from_str::<GameserverMessage>(&json).unwrap(),
            msg
        );
    }

    #[test]
    fn lobby_messages_use_tagged_camel_case_json() {
        let msg = LobbyMessage::Offer {
            request_id: 1,
            sdp: "v=0".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"offer","requestId":1,"sdp":"v=0"}"#);
        assert_eq!(serde_json::from_str::<LobbyMessage>(&json).unwrap(), msg);
    }
}
