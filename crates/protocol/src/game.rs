//! Binary messages exchanged over WebRTC data channels, encoded with postcard.
//!
//! The browser creates two data channels before it makes its offer:
//! * [`CONTROL_CHANNEL`] (reliable, ordered): handshake, roster changes and kills.
//! * [`STATE_CHANNEL`] (unreliable, unordered): inputs from the client every tick and snapshots
//!   from the server. A lost packet does not matter because the next one supersedes it.
//!
//! Field names are camelCase on purpose: postcard ignores them, and the client receives these
//! types as plain JavaScript objects (through `sim-wasm`), typed by the generated definitions.

use core::ops::{BitOr, BitOrAssign};

use serde::{Deserialize, Serialize};

/// Bumped on every incompatible change; the server rejects clients speaking another version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Label of the reliable, ordered data channel.
pub const CONTROL_CHANNEL: &str = "control";

/// Label of the unreliable, unordered data channel.
pub const STATE_CHANNEL: &str = "state";

/// Longest player name the server keeps; longer names are truncated.
pub const MAX_PLAYER_NAME_LEN: usize = 16;

/// How many of its most recent inputs a client repeats in every [`ClientMessage::Inputs`],
/// so that a single lost packet never loses an input.
pub const INPUT_REDUNDANCY: usize = 4;

/// Identifies a player for the lifetime of their connection to a gameserver.
pub type PlayerId = u16;

/// Errors raised when decoding a message.
pub type CodecError = postcard::Error;

/// Buttons held during one simulation tick, as a bit set.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(transparent)]
pub struct Buttons(pub u16);

impl Buttons {
    pub const NONE: Self = Self(0);
    pub const FORWARD: Self = Self(1);
    pub const BACKWARD: Self = Self(1 << 1);
    pub const TURN_LEFT: Self = Self(1 << 2);
    pub const TURN_RIGHT: Self = Self(1 << 3);
    pub const STRAFE_LEFT: Self = Self(1 << 4);
    pub const STRAFE_RIGHT: Self = Self(1 << 5);
    pub const FIRE: Self = Self(1 << 6);
    pub const RELOAD: Self = Self(1 << 7);
    pub const SELECT_RIFLE: Self = Self(1 << 8);
    pub const SELECT_PISTOL: Self = Self(1 << 9);

    /// Whether every button of `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Buttons {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Buttons {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Messages from a client to the gameserver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// First message on the control channel.
    #[serde(rename_all = "camelCase")]
    Hello { protocol: u16, name: String },
    /// Sent on the state channel every tick: the inputs numbered `first_seq..`, oldest first.
    /// Sequence numbers start at 1 and increase by one per tick.
    #[serde(rename_all = "camelCase")]
    Inputs {
        first_seq: u32,
        buttons: Vec<Buttons>,
    },
}

/// Messages from the gameserver to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
pub enum ServerMessage {
    /// Accepts the client's [`ClientMessage::Hello`].
    Welcome(Welcome),
    /// Refuses the client's [`ClientMessage::Hello`]; the server then closes the connection.
    Rejected {
        reason: String,
    },
    PlayerJoined(PlayerInfo),
    PlayerLeft {
        id: PlayerId,
    },
    Kill {
        killer: PlayerId,
        victim: PlayerId,
    },
    /// Sent on the state channel.
    Snapshot(Snapshot),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Welcome {
    /// The id the receiving client plays as.
    pub player_id: PlayerId,
    /// Simulation ticks per second; clients send one input per tick.
    pub tick_rate: u16,
    /// Everyone connected, the receiving client included.
    pub players: Vec<PlayerInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
pub struct PlayerInfo {
    pub id: PlayerId,
    pub name: String,
}

/// The authoritative state of the world after a tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub tick: u32,
    /// The last input sequence number of the receiving client the server has applied (0 if none),
    /// so the client can replay the inputs that came after it on top of this state.
    pub ack_seq: u32,
    pub players: Vec<PlayerState>,
    pub bullets: Vec<BulletState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
pub struct PlayerState {
    pub id: PlayerId,
    pub x: f32,
    pub y: f32,
    /// Radians; 0 faces +x and angles grow clockwise on screen (y points down).
    pub rotation: f32,
    pub health: u8,
    pub weapon: WeaponKind,
    pub alive: bool,
    pub moving: bool,
    pub reloading: bool,
    /// Counts shots fired, wrapping; clients play the shooting animation when it changes.
    pub shots: u8,
    pub mag: u8,
    pub reserve: u16,
    pub kills: u16,
    pub deaths: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
pub struct BulletState {
    pub id: u16,
    pub x: f32,
    pub y: f32,
    pub rotation: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum WeaponKind {
    Rifle,
    Pistol,
}

/// Encodes a message for a data channel.
#[must_use]
pub fn encode<T: Serialize>(message: &T) -> Vec<u8> {
    postcard::to_allocvec(message).expect("protocol messages always serialize")
}

/// Decodes a message received on a data channel.
///
/// # Errors
/// Fails when `bytes` is not a valid encoding of `T`.
pub fn decode<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, CodecError> {
    postcard::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: PlayerId) -> PlayerState {
        PlayerState {
            id,
            x: 640.5,
            y: 360.25,
            rotation: -1.5,
            health: 100,
            weapon: WeaponKind::Rifle,
            alive: true,
            moving: true,
            reloading: false,
            shots: 255,
            mag: 30,
            reserve: 90,
            kills: 3,
            deaths: 1,
        }
    }

    #[test]
    fn client_messages_round_trip() {
        for message in [
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                name: "Ferret".into(),
            },
            ClientMessage::Inputs {
                first_seq: 41,
                buttons: vec![
                    Buttons::FORWARD,
                    Buttons::FORWARD | Buttons::FIRE,
                    Buttons::NONE,
                ],
            },
        ] {
            assert_eq!(decode::<ClientMessage>(&encode(&message)).unwrap(), message);
        }
    }

    #[test]
    fn server_messages_round_trip() {
        let snapshot = Snapshot {
            tick: 1234,
            ack_seq: 1200,
            players: vec![player(1), player(2)],
            bullets: vec![BulletState {
                id: 9,
                x: 1.0,
                y: 2.0,
                rotation: 3.0,
            }],
        };
        for message in [
            ServerMessage::Welcome(Welcome {
                player_id: 1,
                tick_rate: 60,
                players: vec![PlayerInfo {
                    id: 1,
                    name: "Ferret".into(),
                }],
            }),
            ServerMessage::Rejected {
                reason: "full".into(),
            },
            ServerMessage::PlayerJoined(PlayerInfo {
                id: 2,
                name: "Otter".into(),
            }),
            ServerMessage::PlayerLeft { id: 2 },
            ServerMessage::Kill {
                killer: 1,
                victim: 2,
            },
            ServerMessage::Snapshot(snapshot),
        ] {
            assert_eq!(decode::<ServerMessage>(&encode(&message)).unwrap(), message);
        }
    }

    #[test]
    fn busy_snapshot_fits_in_one_datagram() {
        // Unreliable messages larger than one SCTP packet get fragmented, and losing any
        // fragment loses the whole message. Stay well below the ~1200 byte WebRTC MTU.
        let snapshot = Snapshot {
            tick: u32::MAX,
            ack_seq: u32::MAX,
            players: (0..16).map(player).collect(),
            bullets: (0..24)
                .map(|id| BulletState {
                    id,
                    x: 1.0,
                    y: 2.0,
                    rotation: 3.0,
                })
                .collect(),
        };
        let size = encode(&ServerMessage::Snapshot(snapshot)).len();
        assert!(size < 1000, "snapshot is {size} bytes");
    }

    #[test]
    fn decoding_garbage_fails() {
        assert!(decode::<ServerMessage>(&[0xff, 0xff, 0xff]).is_err());
    }
}
