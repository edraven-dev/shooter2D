//! The simulation and the wire codec, compiled to WebAssembly for the browser client.
//!
//! The client encodes and decodes data-channel messages here, so both ends share one codec,
//! and predicts its own movement with [`Predictor`], which runs the gameserver's exact code.

use protocol::game::{
    self, Buttons, CONTROL_CHANNEL, ClientMessage, INPUT_REDUNDANCY, PROTOCOL_VERSION,
    STATE_CHANNEL, ServerMessage,
};
use sim::{Body, Level, Vec2};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(message: &str);
}

/// Reports Rust panics on the browser console instead of an opaque "unreachable" trap.
#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(|info| console_error(&info.to_string())));
}

/// Input buttons; combine with `|` and pass the result as one tick's input.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Forward = 1,
    Backward = 2,
    TurnLeft = 4,
    TurnRight = 8,
    StrafeLeft = 16,
    StrafeRight = 32,
    Fire = 64,
    Reload = 128,
    SelectRifle = 256,
    SelectPistol = 512,
}

#[wasm_bindgen(js_name = protocolVersion)]
#[must_use]
pub fn protocol_version() -> u16 {
    PROTOCOL_VERSION
}

/// Label of the reliable, ordered data channel.
#[wasm_bindgen(js_name = controlChannelLabel)]
#[must_use]
pub fn control_channel_label() -> String {
    CONTROL_CHANNEL.to_owned()
}

/// Label of the unreliable, unordered data channel.
#[wasm_bindgen(js_name = stateChannelLabel)]
#[must_use]
pub fn state_channel_label() -> String {
    STATE_CHANNEL.to_owned()
}

/// How many recent inputs every input message repeats.
#[wasm_bindgen(js_name = inputRedundancy)]
#[must_use]
pub fn input_redundancy() -> u32 {
    INPUT_REDUNDANCY as u32
}

#[wasm_bindgen(js_name = encodeHello)]
#[must_use]
pub fn encode_hello(name: &str) -> Vec<u8> {
    game::encode(&ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        name: name.to_owned(),
    })
}

/// Encodes the inputs numbered `firstSeq..` (oldest first).
#[wasm_bindgen(js_name = encodeInputs)]
#[must_use]
pub fn encode_inputs(first_seq: u32, buttons: &[u16]) -> Vec<u8> {
    let buttons = buttons.iter().copied().map(Buttons).collect();
    game::encode(&ClientMessage::Inputs { first_seq, buttons })
}

/// Decodes a message from the gameserver into a plain object shaped like the generated
/// `ServerMessage` type.
///
/// # Errors
/// Fails when the bytes are not a valid message.
#[wasm_bindgen(js_name = decodeServerMessage)]
pub fn decode_server_message(bytes: &[u8]) -> Result<JsValue, JsError> {
    let message: ServerMessage = game::decode(bytes)?;
    Ok(serde_wasm_bindgen::to_value(&message)?)
}

#[wasm_bindgen(js_name = levelWidth)]
#[must_use]
pub fn level_width() -> f32 {
    Level::arena().width()
}

#[wasm_bindgen(js_name = levelHeight)]
#[must_use]
pub fn level_height() -> f32 {
    Level::arena().height()
}

/// The arena's walls as `[x, y, width, height]` quadruples (top-left corners).
#[wasm_bindgen(js_name = levelWalls)]
#[must_use]
pub fn level_walls() -> Vec<f32> {
    Level::arena()
        .walls()
        .iter()
        .flat_map(|wall| [wall.x, wall.y, wall.width, wall.height])
        .collect()
}

/// Predicts the local player's movement with the gameserver's code.
///
/// Reset it to every authoritative state the server sends, then replay the inputs the server
/// has not acknowledged yet, then keep stepping it once per tick.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Predictor {
    level: Level,
    body: Body,
}

impl Default for Predictor {
    fn default() -> Self {
        Self {
            level: Level::arena(),
            body: Body {
                position: Vec2::ZERO,
                rotation: 0.0,
            },
        }
    }
}

#[wasm_bindgen]
impl Predictor {
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self, x: f32, y: f32, rotation: f32) {
        self.body = Body {
            position: Vec2::new(x, y),
            rotation,
        };
    }

    /// Applies one tick of input.
    pub fn step(&mut self, buttons: u16) {
        self.body = sim::move_body(&self.level, self.body, Buttons(buttons)).0;
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn x(&self) -> f32 {
        self.body.position.x
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn y(&self) -> f32 {
        self.body.position.y
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn rotation(&self) -> f32 {
        self.body.rotation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_match_the_protocol() {
        let pairs = [
            (Button::Forward, Buttons::FORWARD),
            (Button::Backward, Buttons::BACKWARD),
            (Button::TurnLeft, Buttons::TURN_LEFT),
            (Button::TurnRight, Buttons::TURN_RIGHT),
            (Button::StrafeLeft, Buttons::STRAFE_LEFT),
            (Button::StrafeRight, Buttons::STRAFE_RIGHT),
            (Button::Fire, Buttons::FIRE),
            (Button::Reload, Buttons::RELOAD),
            (Button::SelectRifle, Buttons::SELECT_RIFLE),
            (Button::SelectPistol, Buttons::SELECT_PISTOL),
        ];
        for (button, buttons) in pairs {
            assert_eq!(button as u16, buttons.0, "{button:?}");
        }
    }

    #[test]
    fn inputs_round_trip_through_the_protocol() {
        let bytes = encode_inputs(7, &[1, 65]);
        let decoded: ClientMessage = game::decode(&bytes).unwrap();
        assert_eq!(
            decoded,
            ClientMessage::Inputs {
                first_seq: 7,
                buttons: vec![Buttons(1), Buttons(65)]
            }
        );
    }

    #[test]
    fn predictor_steps_like_the_server() {
        let mut predictor = Predictor::new();
        predictor.reset(100.0, 360.0, 0.0);
        predictor.step(Button::Forward as u16);
        let level = Level::arena();
        let expected = sim::move_body(
            &level,
            Body {
                position: Vec2::new(100.0, 360.0),
                rotation: 0.0,
            },
            Buttons::FORWARD,
        )
        .0;
        assert_eq!(
            (predictor.x(), predictor.y(), predictor.rotation()),
            (expected.position.x, expected.position.y, expected.rotation)
        );
    }

    #[test]
    fn walls_are_flattened_in_order() {
        let walls = level_walls();
        assert_eq!(walls.len(), Level::arena().walls().len() * 4);
        assert_eq!(&walls[..4], &[0.0, 0.0, 1280.0, 16.0]);
    }
}
