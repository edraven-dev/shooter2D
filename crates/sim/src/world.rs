use std::collections::BTreeMap;

use protocol::game::{BulletState, Buttons, PlayerId, PlayerState, Snapshot, WeaponKind};
use rapier2d::geometry::Ray;
use rapier2d::math::Pose;
use rapier2d::parry::query::RayCast;
use rapier2d::parry::shape::Ball;

use crate::body::wrap_angle;
use crate::weapon::Ammo;
use crate::{
    BULLET_LIFETIME_TICKS, BULLET_SPEED, Body, DT, Level, MAX_HEALTH, MUZZLE_FORWARD, MUZZLE_RIGHT,
    PLAYER_RADIUS, RESPAWN_TICKS, Vec2, WeaponSpec, move_body,
};

/// Something that happened during a tick that clients are told about reliably.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Kill { killer: PlayerId, victim: PlayerId },
}

/// The authoritative game state of one match.
///
/// Every player advances through their own inputs: [`World::apply_input`] moves them and runs
/// their weapon timers by one tick. [`World::step`] advances everything else (bullets, respawns)
/// once per server tick.
#[derive(Debug)]
pub struct World {
    level: Level,
    tick: u32,
    players: BTreeMap<PlayerId, Player>,
    bullets: Vec<Bullet>,
    next_bullet_id: u16,
    events: Vec<Event>,
}

#[derive(Debug, Clone)]
struct Player {
    body: Body,
    health: u8,
    /// Ticks until respawn; `Some` while dead.
    respawn_in: Option<u16>,
    moving: bool,
    weapon: WeaponKind,
    rifle: Ammo,
    pistol: Ammo,
    fire_cooldown: u16,
    /// Ticks until the running reload completes; 0 when not reloading.
    reload_left: u16,
    shots: u8,
    previous_buttons: Buttons,
    kills: u16,
    deaths: u16,
}

#[derive(Debug, Clone)]
struct Bullet {
    id: u16,
    owner: PlayerId,
    position: Vec2,
    direction: Vec2,
    rotation: f32,
    damage: u8,
    ticks_left: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Wall,
    Player(PlayerId),
}

impl World {
    #[must_use]
    pub fn new(level: Level) -> Self {
        Self {
            level,
            tick: 0,
            players: BTreeMap::new(),
            bullets: Vec::new(),
            next_bullet_id: 0,
            events: Vec::new(),
        }
    }

    #[must_use]
    pub fn level(&self) -> &Level {
        &self.level
    }

    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick
    }

    #[must_use]
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// The body of a player, if they are in the world.
    #[must_use]
    pub fn body(&self, id: PlayerId) -> Option<Body> {
        self.players.get(&id).map(|player| player.body)
    }

    /// Adds a player at the spawn point farthest from everyone else. Does nothing if `id`
    /// is already playing.
    pub fn spawn_player(&mut self, id: PlayerId) {
        if !self.players.contains_key(&id) {
            let body = self.spawn_body();
            self.players.insert(id, Player::new(body));
        }
    }

    /// Removes a player and the bullets they fired.
    pub fn remove_player(&mut self, id: PlayerId) {
        self.players.remove(&id);
        self.bullets.retain(|bullet| bullet.owner != id);
    }

    /// Runs one tick of a player's input: movement, weapon timers and weapon buttons.
    pub fn apply_input(&mut self, id: PlayerId, buttons: Buttons) {
        let Some(player) = self.players.get_mut(&id) else {
            return;
        };
        let previous = core::mem::replace(&mut player.previous_buttons, buttons);
        if !player.alive() {
            return;
        }
        let (body, moving) = move_body(&self.level, player.body, buttons);
        player.body = body;
        player.moving = moving;
        if player.update_weapon(buttons, previous) {
            let damage = WeaponSpec::of(player.weapon).damage;
            self.fire(id, body, damage);
        }
    }

    /// Advances bullets and respawn timers by one tick.
    pub fn step(&mut self) {
        self.tick = self.tick.wrapping_add(1);

        let travel = BULLET_SPEED * DT;
        for mut bullet in core::mem::take(&mut self.bullets) {
            match self.trace(bullet.position, bullet.direction, travel, bullet.owner) {
                Some((Target::Wall, _)) => {}
                Some((Target::Player(victim), _)) => self.hit(victim, bullet.owner, bullet.damage),
                None => {
                    bullet.position += bullet.direction * travel;
                    bullet.ticks_left -= 1;
                    if bullet.ticks_left > 0 {
                        self.bullets.push(bullet);
                    }
                }
            }
        }

        let mut respawning = Vec::new();
        for (&id, player) in &mut self.players {
            if let Some(ticks) = &mut player.respawn_in {
                *ticks = ticks.saturating_sub(1);
                if *ticks == 0 {
                    respawning.push(id);
                }
            }
        }
        for id in respawning {
            let body = self.spawn_body();
            if let Some(player) = self.players.get_mut(&id) {
                player.respawn(body);
            }
        }
    }

    /// Takes the events that happened since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }

    /// The current state of the world, for a client whose last applied input is `ack_seq`.
    #[must_use]
    pub fn snapshot(&self, ack_seq: u32) -> Snapshot {
        Snapshot {
            tick: self.tick,
            ack_seq,
            players: self
                .players
                .iter()
                .map(|(&id, player)| player.state(id))
                .collect(),
            bullets: self
                .bullets
                .iter()
                .map(|bullet| BulletState {
                    id: bullet.id,
                    x: bullet.position.x,
                    y: bullet.position.y,
                    rotation: bullet.rotation,
                })
                .collect(),
        }
    }

    /// The spawn point farthest from every living player (the first one on ties), facing the
    /// center of the level.
    fn spawn_body(&self) -> Body {
        let distance_to_nearest_player = |point: Vec2| {
            self.players
                .values()
                .filter(|player| player.alive())
                .map(|player| player.body.position.distance(point))
                .fold(f32::INFINITY, f32::min)
        };
        let spawn_points = self.level.spawn_points();
        let mut best = spawn_points[0];
        let mut best_distance = distance_to_nearest_player(best);
        for &point in &spawn_points[1..] {
            let distance = distance_to_nearest_player(point);
            if distance > best_distance {
                (best, best_distance) = (point, distance);
            }
        }
        let to_center = self.level.center() - best;
        Body {
            position: best,
            rotation: wrap_angle(libm::atan2f(to_center.y, to_center.x)),
        }
    }

    fn fire(&mut self, shooter: PlayerId, body: Body, damage: u8) {
        let muzzle = body.forward() * MUZZLE_FORWARD + body.right() * MUZZLE_RIGHT;
        let reach = muzzle.length();
        // A wall or a player between the shooter and the muzzle takes the shot point-blank.
        match self.trace(body.position, muzzle / reach, reach, shooter) {
            Some((Target::Wall, _)) => {}
            Some((Target::Player(victim), _)) => self.hit(victim, shooter, damage),
            None => {
                let id = self.next_bullet_id;
                self.next_bullet_id = id.wrapping_add(1);
                self.bullets.push(Bullet {
                    id,
                    owner: shooter,
                    position: body.position + muzzle,
                    direction: body.forward(),
                    rotation: body.rotation,
                    damage,
                    ticks_left: BULLET_LIFETIME_TICKS,
                });
            }
        }
    }

    /// The nearest wall or living player (other than `ignore`) on a segment.
    fn trace(
        &self,
        origin: Vec2,
        direction: Vec2,
        max_distance: f32,
        ignore: PlayerId,
    ) -> Option<(Target, f32)> {
        let mut nearest = self
            .level
            .cast_ray(origin, direction, max_distance)
            .map(|d| (Target::Wall, d));
        let ray = Ray::new(origin, direction);
        let ball = Ball::new(PLAYER_RADIUS);
        for (&id, player) in &self.players {
            if id == ignore || !player.alive() {
                continue;
            }
            let limit = nearest.map_or(max_distance, |(_, distance)| distance);
            if let Some(distance) = ball.cast_ray(
                &Pose::from_translation(player.body.position),
                &ray,
                limit,
                true,
            ) && distance < limit
            {
                nearest = Some((Target::Player(id), distance));
            }
        }
        nearest
    }

    fn hit(&mut self, victim: PlayerId, attacker: PlayerId, damage: u8) {
        let Some(player) = self.players.get_mut(&victim) else {
            return;
        };
        if !player.alive() {
            return;
        }
        player.health = player.health.saturating_sub(damage);
        if player.health > 0 {
            return;
        }
        player.die();
        if attacker != victim
            && let Some(killer) = self.players.get_mut(&attacker)
        {
            killer.kills = killer.kills.saturating_add(1);
        }
        self.events.push(Event::Kill {
            killer: attacker,
            victim,
        });
    }
}

impl Player {
    fn new(body: Body) -> Self {
        Self {
            body,
            health: MAX_HEALTH,
            respawn_in: None,
            moving: false,
            weapon: WeaponKind::Rifle,
            rifle: Ammo::full(WeaponKind::Rifle),
            pistol: Ammo::full(WeaponKind::Pistol),
            fire_cooldown: 0,
            reload_left: 0,
            shots: 0,
            previous_buttons: Buttons::NONE,
            kills: 0,
            deaths: 0,
        }
    }

    fn alive(&self) -> bool {
        self.respawn_in.is_none()
    }

    fn ammo(&self) -> Ammo {
        match self.weapon {
            WeaponKind::Rifle => self.rifle,
            WeaponKind::Pistol => self.pistol,
        }
    }

    fn ammo_mut(&mut self) -> &mut Ammo {
        match self.weapon {
            WeaponKind::Rifle => &mut self.rifle,
            WeaponKind::Pistol => &mut self.pistol,
        }
    }

    /// Runs the weapon timers for one tick and handles the weapon buttons.
    /// Returns whether the player fired.
    fn update_weapon(&mut self, buttons: Buttons, previous: Buttons) -> bool {
        self.fire_cooldown = self.fire_cooldown.saturating_sub(1);
        if self.reload_left > 0 {
            self.reload_left -= 1;
            if self.reload_left == 0 {
                let weapon = self.weapon;
                self.ammo_mut().reload(weapon);
            }
        }

        let selected = if buttons.contains(Buttons::SELECT_RIFLE) {
            Some(WeaponKind::Rifle)
        } else if buttons.contains(Buttons::SELECT_PISTOL) {
            Some(WeaponKind::Pistol)
        } else {
            None
        };
        if let Some(weapon) = selected
            && weapon != self.weapon
        {
            self.weapon = weapon;
            self.reload_left = 0;
        }

        if buttons.contains(Buttons::RELOAD) {
            self.start_reload();
        }

        let spec = WeaponSpec::of(self.weapon);
        let trigger = buttons.contains(Buttons::FIRE)
            && (spec.automatic || !previous.contains(Buttons::FIRE));
        if !trigger || self.reload_left > 0 || self.fire_cooldown > 0 {
            return false;
        }
        if self.ammo().mag == 0 {
            self.start_reload();
            return false;
        }
        self.ammo_mut().mag -= 1;
        self.fire_cooldown = spec.fire_interval;
        self.shots = self.shots.wrapping_add(1);
        true
    }

    fn start_reload(&mut self) {
        if self.reload_left == 0 && self.ammo().can_reload(self.weapon) {
            self.reload_left = WeaponSpec::of(self.weapon).reload_ticks;
        }
    }

    fn die(&mut self) {
        self.respawn_in = Some(RESPAWN_TICKS);
        self.moving = false;
        self.reload_left = 0;
        self.deaths = self.deaths.saturating_add(1);
    }

    fn respawn(&mut self, body: Body) {
        *self = Self {
            shots: self.shots,
            previous_buttons: self.previous_buttons,
            kills: self.kills,
            deaths: self.deaths,
            ..Self::new(body)
        };
    }

    fn state(&self, id: PlayerId) -> PlayerState {
        let ammo = self.ammo();
        PlayerState {
            id,
            x: self.body.position.x,
            y: self.body.position.y,
            rotation: self.body.rotation,
            health: self.health,
            weapon: self.weapon,
            alive: self.alive(),
            moving: self.moving,
            reloading: self.reload_left > 0,
            shots: self.shots,
            mag: ammo.mag,
            reserve: ammo.reserve,
            kills: self.kills,
            deaths: self.deaths,
        }
    }
}

#[cfg(test)]
mod tests {
    use core::f32::consts::PI;

    use super::*;

    const SHOOTER: PlayerId = 1;
    const TARGET: PlayerId = 2;

    fn state(world: &World, id: PlayerId) -> PlayerState {
        world
            .snapshot(0)
            .players
            .into_iter()
            .find(|player| player.id == id)
            .unwrap()
    }

    /// Two players facing each other across open floor in the arena's top lane.
    fn duel() -> World {
        let mut world = World::new(Level::arena());
        world.spawn_player(SHOOTER);
        world.spawn_player(TARGET);
        world.players.get_mut(&SHOOTER).unwrap().body = Body {
            position: Vec2::new(500.0, 88.0),
            rotation: 0.0,
        };
        world.players.get_mut(&TARGET).unwrap().body = Body {
            position: Vec2::new(800.0, 100.0),
            rotation: PI - 0.01,
        };
        world
    }

    /// Holds `buttons` for `ticks` ticks (one input and one world step per tick).
    fn hold(world: &mut World, id: PlayerId, buttons: Buttons, ticks: usize) {
        for _ in 0..ticks {
            world.apply_input(id, buttons);
            world.step();
        }
    }

    #[test]
    fn first_player_spawns_at_the_first_spawn_point_facing_the_center() {
        let mut world = World::new(Level::arena());
        world.spawn_player(SHOOTER);
        let body = world.body(SHOOTER).unwrap();
        assert_eq!(body.position, world.level().spawn_points()[0]);
        let to_center = (world.level().center() - body.position).normalize();
        assert!(body.forward().distance(to_center) < 1.0e-5);
    }

    #[test]
    fn players_spawn_away_from_each_other() {
        let mut world = World::new(Level::arena());
        world.spawn_player(SHOOTER);
        world.spawn_player(TARGET);
        let distance = world
            .body(SHOOTER)
            .unwrap()
            .position
            .distance(world.body(TARGET).unwrap().position);
        assert!(distance > 1000.0, "spawned {distance}px apart");
    }

    #[test]
    fn rifle_fires_automatically_at_its_fire_rate() {
        let mut world = duel();
        hold(&mut world, SHOOTER, Buttons::FIRE, 60);
        let shooter = state(&world, SHOOTER);
        assert_eq!(shooter.shots, 10);
        assert_eq!(shooter.mag, 20);
    }

    #[test]
    fn pistol_needs_a_new_press_for_every_shot() {
        let mut world = duel();
        hold(&mut world, SHOOTER, Buttons::SELECT_PISTOL, 1);
        hold(&mut world, SHOOTER, Buttons::FIRE, 60);
        assert_eq!(state(&world, SHOOTER).shots, 1);
        for _ in 0..3 {
            hold(&mut world, SHOOTER, Buttons::NONE, 6);
            hold(&mut world, SHOOTER, Buttons::FIRE, 1);
        }
        let shooter = state(&world, SHOOTER);
        assert_eq!(shooter.shots, 4);
        assert_eq!(shooter.weapon, WeaponKind::Pistol);
        assert_eq!(shooter.mag, 5);
    }

    #[test]
    fn firing_an_empty_magazine_reloads_it() {
        let mut world = duel();
        hold(&mut world, SHOOTER, Buttons::SELECT_PISTOL, 1);
        for _ in 0..9 {
            hold(&mut world, SHOOTER, Buttons::FIRE, 1);
            hold(&mut world, SHOOTER, Buttons::NONE, 6);
        }
        assert_eq!(state(&world, SHOOTER).mag, 0);
        hold(&mut world, SHOOTER, Buttons::FIRE, 1);
        assert!(state(&world, SHOOTER).reloading);
        hold(
            &mut world,
            SHOOTER,
            Buttons::NONE,
            usize::from(WeaponSpec::of(WeaponKind::Pistol).reload_ticks),
        );
        let shooter = state(&world, SHOOTER);
        assert!(!shooter.reloading);
        assert_eq!((shooter.mag, shooter.reserve), (9, 27));
    }

    #[test]
    fn switching_weapons_cancels_a_reload() {
        let mut world = duel();
        hold(&mut world, SHOOTER, Buttons::FIRE, 1);
        hold(&mut world, SHOOTER, Buttons::RELOAD, 1);
        assert!(state(&world, SHOOTER).reloading);
        hold(&mut world, SHOOTER, Buttons::SELECT_PISTOL, 1);
        hold(&mut world, SHOOTER, Buttons::SELECT_RIFLE, 60);
        let shooter = state(&world, SHOOTER);
        assert!(!shooter.reloading);
        assert_eq!(shooter.mag, 29);
    }

    #[test]
    fn bullets_damage_and_kill_players_who_then_respawn() {
        let mut world = duel();
        // 7 rifle hits of 15 damage kill a player with 100 health.
        let mut ticks = 0;
        while state(&world, TARGET).alive {
            hold(&mut world, SHOOTER, Buttons::FIRE, 1);
            ticks += 1;
            assert!(ticks < 120, "the target never died");
        }
        let events = world.take_events();
        assert_eq!(
            events,
            vec![Event::Kill {
                killer: SHOOTER,
                victim: TARGET
            }]
        );
        assert_eq!(state(&world, SHOOTER).kills, 1);
        assert_eq!(state(&world, TARGET).deaths, 1);

        // Dead players ignore input.
        let corpse = world.body(TARGET).unwrap();
        hold(&mut world, TARGET, Buttons::FORWARD, 1);
        assert_eq!(world.body(TARGET).unwrap(), corpse);

        hold(
            &mut world,
            SHOOTER,
            Buttons::NONE,
            usize::from(RESPAWN_TICKS),
        );
        let target = state(&world, TARGET);
        assert!(target.alive);
        assert_eq!(target.health, MAX_HEALTH);
        let distance = world
            .body(TARGET)
            .unwrap()
            .position
            .distance(world.body(SHOOTER).unwrap().position);
        assert!(distance > 500.0, "respawned {distance}px from the killer");
    }

    #[test]
    fn walls_stop_bullets() {
        let mut world = duel();
        // Face the top border wall and fire.
        world.players.get_mut(&SHOOTER).unwrap().body.rotation = -PI / 2.0;
        hold(&mut world, SHOOTER, Buttons::FIRE, 1);
        hold(&mut world, SHOOTER, Buttons::NONE, 10);
        assert!(world.snapshot(0).bullets.is_empty());
        assert_eq!(state(&world, TARGET).health, MAX_HEALTH);
    }

    #[test]
    fn removing_a_player_removes_their_bullets() {
        let mut world = duel();
        world.players.get_mut(&SHOOTER).unwrap().body.rotation = PI / 2.0;
        hold(&mut world, SHOOTER, Buttons::FIRE, 1);
        assert_eq!(world.snapshot(0).bullets.len(), 1);
        world.remove_player(SHOOTER);
        assert!(world.snapshot(0).bullets.is_empty());
        assert_eq!(world.player_count(), 1);
    }
}
