//! Deterministic game simulation, shared by the gameserver and the browser client.
//!
//! The gameserver runs the authoritative [`World`]. The client compiles this crate to WASM (via
//! `sim-wasm`) and calls the same [`move_body`] to predict its own player between snapshots.
//! Physics runs on Rapier with `enhanced-determinism`, and trigonometry uses `libm`, so the
//! native server and the WASM client compute bit-identical movement for the same inputs.
//!
//! Units are pixels and seconds; y points down, and angles grow clockwise on screen.
#![forbid(unsafe_code)]

mod body;
mod level;
mod weapon;
mod world;

pub use body::{Body, move_body};
pub use level::{Level, Rect};
pub use rapier2d::math::Vector as Vec2;
pub use weapon::WeaponSpec;
pub use world::{Event, World};

/// Simulation ticks per second.
pub const TICK_RATE: u16 = 60;
/// Seconds per tick.
pub const DT: f32 = 1.0 / TICK_RATE as f32;

/// Radius of a player's collision circle.
pub const PLAYER_RADIUS: f32 = 22.0;
/// Forward and backward speed, in pixels per second.
pub const MOVE_SPEED: f32 = 180.0;
/// Sideways speed, in pixels per second.
pub const STRAFE_SPEED: f32 = 90.0;
/// Turning speed, in radians per second.
pub const TURN_SPEED: f32 = core::f32::consts::PI;
pub const MAX_HEALTH: u8 = 100;
/// Ticks a dead player waits before respawning.
pub const RESPAWN_TICKS: u16 = 3 * TICK_RATE;

/// Bullet speed, in pixels per second.
pub const BULLET_SPEED: f32 = 720.0;
/// Ticks after which a bullet that hit nothing disappears.
pub const BULLET_LIFETIME_TICKS: u16 = 2 * TICK_RATE;
/// Where bullets leave the gun, relative to the player's center: this far forward…
pub const MUZZLE_FORWARD: f32 = 35.0;
/// …and this far to the right (the sprite holds the gun in its right hand).
pub const MUZZLE_RIGHT: f32 = 12.0;
