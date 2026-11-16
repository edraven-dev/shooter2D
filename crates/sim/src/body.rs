use core::f32::consts::{PI, TAU};

use protocol::game::Buttons;

use crate::{DT, Level, MOVE_SPEED, STRAFE_SPEED, TURN_SPEED, Vec2};

/// Where a player is and where they face: the part of the state clients predict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub position: Vec2,
    /// Radians in `[-π, π)`.
    pub rotation: f32,
}

impl Body {
    /// Unit vector the body faces.
    #[must_use]
    pub fn forward(&self) -> Vec2 {
        Vec2::new(libm::cosf(self.rotation), libm::sinf(self.rotation))
    }

    /// Unit vector to the body's right (y points down, so this is `forward` turned clockwise).
    #[must_use]
    pub fn right(&self) -> Vec2 {
        let forward = self.forward();
        Vec2::new(-forward.y, forward.x)
    }
}

/// Advances a body by one tick of input, returning it and whether it tried to move.
///
/// The gameserver and client-side prediction both call this; for the same level, body and
/// buttons they get bit-identical results (Rapier's `enhanced-determinism` plus `libm`).
#[must_use]
pub fn move_body(level: &Level, body: Body, buttons: Buttons) -> (Body, bool) {
    let mut rotation = body.rotation;
    if buttons.contains(Buttons::TURN_LEFT) {
        rotation -= TURN_SPEED * DT;
    }
    if buttons.contains(Buttons::TURN_RIGHT) {
        rotation += TURN_SPEED * DT;
    }
    let turned = Body {
        position: body.position,
        rotation: wrap_angle(rotation),
    };

    let (forward, right) = (turned.forward(), turned.right());
    let mut velocity = Vec2::ZERO;
    if buttons.contains(Buttons::FORWARD) {
        velocity += forward * MOVE_SPEED;
    }
    if buttons.contains(Buttons::BACKWARD) {
        velocity -= forward * MOVE_SPEED;
    }
    if buttons.contains(Buttons::STRAFE_LEFT) {
        velocity -= right * STRAFE_SPEED;
    }
    if buttons.contains(Buttons::STRAFE_RIGHT) {
        velocity += right * STRAFE_SPEED;
    }

    let moving = velocity != Vec2::ZERO;
    let position = if moving {
        level.move_and_slide(turned.position, velocity * DT)
    } else {
        turned.position
    };
    (
        Body {
            position,
            rotation: turned.rotation,
        },
        moving,
    )
}

/// Wraps an angle that is at most one turn outside `[-π, π)` back into it.
pub(crate) fn wrap_angle(angle: f32) -> f32 {
    if angle >= PI {
        angle - TAU
    } else if angle < -PI {
        angle + TAU
    } else {
        angle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Vec2, b: Vec2) -> bool {
        a.distance(b) < 1.0e-3
    }

    #[test]
    fn moves_forward_in_the_facing_direction() {
        let level = Level::arena();
        let body = Body {
            position: Vec2::new(100.0, 360.0),
            rotation: 0.0,
        };
        let (moved, moving) = move_body(&level, body, Buttons::FORWARD);
        assert!(moving);
        assert!(approx(moved.position, Vec2::new(103.0, 360.0)), "{moved:?}");
    }

    #[test]
    fn strafes_right_relative_to_facing() {
        let level = Level::arena();
        // Facing up (-y): right is +x.
        let body = Body {
            position: Vec2::new(100.0, 360.0),
            rotation: -PI / 2.0,
        };
        let (moved, _) = move_body(&level, body, Buttons::STRAFE_RIGHT);
        assert!(approx(moved.position, Vec2::new(101.5, 360.0)), "{moved:?}");
    }

    #[test]
    fn turning_alone_is_not_moving() {
        let level = Level::arena();
        let body = Body {
            position: Vec2::new(100.0, 360.0),
            rotation: 0.0,
        };
        let (turned, moving) = move_body(&level, body, Buttons::TURN_RIGHT);
        assert!(!moving);
        assert_eq!(turned.position, body.position);
        assert!((turned.rotation - TURN_SPEED * DT).abs() < 1.0e-6);
    }

    #[test]
    fn rotation_stays_wrapped() {
        let level = Level::arena();
        let mut body = Body {
            position: Vec2::new(100.0, 360.0),
            rotation: 0.0,
        };
        for _ in 0..1000 {
            body = move_body(&level, body, Buttons::TURN_LEFT).0;
            assert!((-PI..PI).contains(&body.rotation), "{}", body.rotation);
        }
    }

    #[test]
    fn replaying_inputs_is_deterministic() {
        let level = Level::arena();
        let inputs: Vec<Buttons> = (0..600u16)
            .map(|i| match i % 7 {
                0 | 1 => Buttons::FORWARD | Buttons::TURN_LEFT,
                2 => Buttons::STRAFE_LEFT,
                3 | 4 => Buttons::FORWARD | Buttons::STRAFE_RIGHT,
                5 => Buttons::BACKWARD | Buttons::TURN_RIGHT,
                _ => Buttons::FORWARD,
            })
            .collect();
        let run = || {
            inputs.iter().fold(
                Body {
                    position: Vec2::new(100.0, 100.0),
                    rotation: 0.0,
                },
                |body, &b| move_body(&level, body, b).0,
            )
        };
        let (a, b) = (run(), run());
        assert_eq!(a.position.x.to_bits(), b.position.x.to_bits());
        assert_eq!(a.position.y.to_bits(), b.position.y.to_bits());
        assert_eq!(a.rotation.to_bits(), b.rotation.to_bits());
        assert!(!level.blocks(a.position));
    }
}
