use core::fmt;

use rapier2d::dynamics::{IntegrationParameters, RigidBodySet};
use rapier2d::geometry::{BroadPhaseBvh, ColliderBuilder, ColliderSet, Ray};
use rapier2d::math::{Pose, Vector};
use rapier2d::parry::query::{DefaultQueryDispatcher, ShapeCastOptions};
use rapier2d::parry::shape::Ball;
use rapier2d::pipeline::{QueryFilter, QueryPipeline};

use crate::PLAYER_RADIUS;

/// Gap kept between a moving player and the walls, so casts never start in contact.
const SKIN: f32 = 0.1;
/// Push away from a wall after sliding along it, so the next cast does not report the same
/// contact again (Rapier's character controller uses the same trick).
const NUDGE: f32 = 1.0e-3;
/// Collisions resolved per move. Two are enough for a corner; the rest is slack.
const MAX_SLIDES: usize = 4;

/// An axis-aligned rectangle; `x`/`y` is its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn center(&self) -> Vector {
        Vector::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// The static geometry of a map: walls that block players and bullets, and spawn points.
///
/// The walls live in their own Rapier collider set, queried through its BVH. Players are not in
/// it, which keeps it identical on the server and in client-side prediction.
pub struct Level {
    width: f32,
    height: f32,
    walls: Vec<Rect>,
    spawn_points: Vec<Vector>,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    broad_phase: BroadPhaseBvh,
}

impl fmt::Debug for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Level")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("walls", &self.walls)
            .field("spawn_points", &self.spawn_points)
            .finish_non_exhaustive()
    }
}

impl Level {
    /// Builds a level. Spawn points must leave room for a player.
    #[must_use]
    pub fn new(width: f32, height: f32, walls: Vec<Rect>, spawn_points: Vec<Vector>) -> Self {
        assert!(
            !spawn_points.is_empty(),
            "a level needs at least one spawn point"
        );
        let params = IntegrationParameters {
            length_unit: 50.0,
            ..IntegrationParameters::default()
        };
        let mut colliders = ColliderSet::new();
        let mut broad_phase = BroadPhaseBvh::new();
        for wall in &walls {
            let handle = colliders.insert(
                ColliderBuilder::cuboid(wall.width / 2.0, wall.height / 2.0)
                    .translation(wall.center()),
            );
            broad_phase.set_aabb(&params, handle, colliders[handle].compute_aabb());
        }
        Self {
            width,
            height,
            walls,
            spawn_points,
            bodies: RigidBodySet::new(),
            colliders,
            broad_phase,
        }
    }

    /// The 1280×720 arena: a border with some cover in the middle.
    #[must_use]
    pub fn arena() -> Self {
        let (width, height, border) = (1280.0, 720.0, 16.0);
        let walls = vec![
            // Border.
            Rect::new(0.0, 0.0, width, border),
            Rect::new(0.0, height - border, width, border),
            Rect::new(0.0, 0.0, border, height),
            Rect::new(width - border, 0.0, border, height),
            // Cover.
            Rect::new(600.0, 320.0, 80.0, 80.0),
            Rect::new(240.0, 160.0, 120.0, 40.0),
            Rect::new(920.0, 160.0, 120.0, 40.0),
            Rect::new(240.0, 520.0, 120.0, 40.0),
            Rect::new(920.0, 520.0, 120.0, 40.0),
            Rect::new(400.0, 280.0, 32.0, 160.0),
            Rect::new(848.0, 280.0, 32.0, 160.0),
        ];
        let spawn_points = [
            (100.0, 100.0),
            (1180.0, 620.0),
            (1180.0, 100.0),
            (100.0, 620.0),
            (640.0, 120.0),
            (640.0, 600.0),
            (100.0, 360.0),
            (1180.0, 360.0),
        ]
        .into_iter()
        .map(|(x, y)| Vector::new(x, y))
        .collect();
        Self::new(width, height, walls, spawn_points)
    }

    #[must_use]
    pub fn width(&self) -> f32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> f32 {
        self.height
    }

    #[must_use]
    pub fn walls(&self) -> &[Rect] {
        &self.walls
    }

    #[must_use]
    pub fn spawn_points(&self) -> &[Vector] {
        &self.spawn_points
    }

    #[must_use]
    pub fn center(&self) -> Vector {
        Vector::new(self.width / 2.0, self.height / 2.0)
    }

    fn queries(&self) -> QueryPipeline<'_> {
        self.broad_phase.as_query_pipeline(
            &DefaultQueryDispatcher,
            &self.bodies,
            &self.colliders,
            QueryFilter::default(),
        )
    }

    /// Moves a player's circle by `translation`, sliding along the walls it runs into.
    /// Returns the new position.
    #[must_use]
    pub fn move_and_slide(&self, start: Vector, translation: Vector) -> Vector {
        let queries = self.queries();
        let shape = Ball::new(PLAYER_RADIUS);
        let mut position = start;
        let mut remaining = translation;

        for _ in 0..MAX_SLIDES {
            let distance = remaining.length();
            if distance <= 1.0e-4 {
                break;
            }
            let direction = remaining / distance;
            let options = ShapeCastOptions {
                max_time_of_impact: distance,
                target_distance: SKIN,
                stop_at_penetration: false,
                compute_impact_geometry_on_penetration: true,
            };
            let Some((_, hit)) = queries.cast_shape(
                &Pose::from_translation(position),
                direction,
                &shape,
                options,
            ) else {
                position += remaining;
                break;
            };

            position += direction * hit.time_of_impact;
            remaining = direction * (distance - hit.time_of_impact);
            // `normal1` points out of the wall: drop the part of the motion going into it.
            let into_wall = remaining.dot(hit.normal1);
            if into_wall < 0.0 {
                remaining -= hit.normal1 * into_wall;
            }
            remaining += hit.normal1 * NUDGE;
        }
        position
    }

    /// Distance from `origin` along the unit vector `direction` to the first wall, if one is
    /// closer than `max_distance`. An origin inside a wall hits it at distance 0.
    #[must_use]
    pub fn cast_ray(&self, origin: Vector, direction: Vector, max_distance: f32) -> Option<f32> {
        self.queries()
            .cast_ray(&Ray::new(origin, direction), max_distance, true)
            .map(|(_, distance)| distance)
    }

    /// Whether a player's circle at `position` overlaps a wall.
    #[must_use]
    pub fn blocks(&self, position: Vector) -> bool {
        self.walls.iter().any(|wall| {
            let closest = Vector::new(
                position.x.clamp(wall.x, wall.x + wall.width),
                position.y.clamp(wall.y, wall.y + wall.height),
            );
            closest.distance(position) < PLAYER_RADIUS
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_spawn_points_are_free() {
        let level = Level::arena();
        for &spawn in level.spawn_points() {
            assert!(
                !level.blocks(spawn),
                "spawn point {spawn:?} overlaps a wall"
            );
        }
    }

    #[test]
    fn free_movement_is_unobstructed() {
        let level = Level::arena();
        let start = Vector::new(100.0, 100.0);
        assert_eq!(
            level.move_and_slide(start, Vector::new(3.0, 0.0)),
            Vector::new(103.0, 100.0)
        );
    }

    #[test]
    fn walls_stop_players() {
        let level = Level::arena();
        // Walk left into the border wall (its inner face is at x = 16) for a while.
        let mut position = Vector::new(60.0, 360.0);
        for _ in 0..100 {
            position = level.move_and_slide(position, Vector::new(-3.0, 0.0));
        }
        assert!(
            (position.x - (16.0 + PLAYER_RADIUS)).abs() < 0.5,
            "x = {}",
            position.x
        );
        assert!((position.y - 360.0).abs() < 0.01, "y = {}", position.y);
        assert!(!level.blocks(position));
    }

    #[test]
    fn players_slide_along_walls() {
        let level = Level::arena();
        // Walk diagonally into the left border: the vertical part of the motion must survive.
        let mut position = Vector::new(40.0, 300.0);
        for _ in 0..20 {
            position = level.move_and_slide(position, Vector::new(-2.0, 2.0));
        }
        assert!(position.y > 335.0, "y = {}", position.y);
        assert!(!level.blocks(position));
    }

    #[test]
    fn corners_hold() {
        let level = Level::arena();
        let mut position = Vector::new(60.0, 60.0);
        for _ in 0..100 {
            position = level.move_and_slide(position, Vector::new(-3.0, -3.0));
        }
        assert!(!level.blocks(position), "{position:?} is inside a wall");
        assert!(
            position.x < 40.0 && position.y < 40.0,
            "{position:?} is not in the corner"
        );
    }

    #[test]
    fn rays_hit_walls() {
        let level = Level::arena();
        let distance = level.cast_ray(Vector::new(100.0, 360.0), Vector::new(-1.0, 0.0), 500.0);
        assert_eq!(distance, Some(84.0));
        assert_eq!(
            level.cast_ray(Vector::new(100.0, 360.0), Vector::new(-1.0, 0.0), 50.0),
            None
        );
    }
}
