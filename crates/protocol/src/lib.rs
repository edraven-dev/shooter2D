//! Wire protocol shared by the lobby, the gameserver and the browser client.
//!
//! * [`lobby`]: JSON messages. The lobby HTTP API used by browsers, and the WebSocket
//!   protocol gameservers use to register with the lobby and receive WebRTC offers.
//! * [`game`]: binary messages (postcard) exchanged between browsers and gameservers over
//!   WebRTC data channels.
//!
//! With the `ts-export` feature, `cargo test -p protocol --features ts-export` regenerates the
//! TypeScript definitions the client imports (see `.cargo/config.toml` for the output folder).
#![forbid(unsafe_code)]

pub mod game;
pub mod lobby;
