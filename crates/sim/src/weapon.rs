use protocol::game::WeaponKind;

use crate::TICK_RATE;

/// Static properties of a weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponSpec {
    pub mag_size: u8,
    /// Spare rounds a player spawns with.
    pub reserve: u16,
    pub damage: u8,
    /// Minimum ticks between two shots.
    pub fire_interval: u16,
    /// Keeps firing while the trigger is held; otherwise every shot needs a new press.
    pub automatic: bool,
    pub reload_ticks: u16,
}

impl WeaponSpec {
    /// Fire intervals and reload times match the length of the sprite animations
    /// (played at 30 frames per second).
    #[must_use]
    pub const fn of(kind: WeaponKind) -> &'static Self {
        const RIFLE: WeaponSpec = WeaponSpec {
            mag_size: 30,
            reserve: 90,
            damage: 15,
            fire_interval: TICK_RATE / 10,
            automatic: true,
            reload_ticks: TICK_RATE * 2 / 3,
        };
        const PISTOL: WeaponSpec = WeaponSpec {
            mag_size: 9,
            reserve: 36,
            damage: 25,
            fire_interval: TICK_RATE / 10,
            automatic: false,
            reload_ticks: TICK_RATE / 2,
        };
        match kind {
            WeaponKind::Rifle => &RIFLE,
            WeaponKind::Pistol => &PISTOL,
        }
    }
}

/// Ammunition a player carries for one weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ammo {
    pub mag: u8,
    pub reserve: u16,
}

impl Ammo {
    pub(crate) fn full(kind: WeaponKind) -> Self {
        let spec = WeaponSpec::of(kind);
        Self {
            mag: spec.mag_size,
            reserve: spec.reserve,
        }
    }

    pub(crate) fn can_reload(self, kind: WeaponKind) -> bool {
        self.mag < WeaponSpec::of(kind).mag_size && self.reserve > 0
    }

    /// Moves rounds from the reserve into the magazine.
    pub(crate) fn reload(&mut self, kind: WeaponKind) {
        let missing = u16::from(WeaponSpec::of(kind).mag_size - self.mag);
        let moved = missing.min(self.reserve);
        self.reserve -= moved;
        // `moved` is at most `missing`, which came from a u8.
        self.mag += u8::try_from(moved).expect("moved rounds fit a magazine");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_fills_the_magazine_from_the_reserve() {
        let mut ammo = Ammo {
            mag: 5,
            reserve: 90,
        };
        ammo.reload(WeaponKind::Rifle);
        assert_eq!(
            ammo,
            Ammo {
                mag: 30,
                reserve: 65
            }
        );
    }

    #[test]
    fn reload_uses_what_is_left() {
        let mut ammo = Ammo { mag: 0, reserve: 4 };
        ammo.reload(WeaponKind::Pistol);
        assert_eq!(ammo, Ammo { mag: 4, reserve: 0 });
        assert!(!ammo.can_reload(WeaponKind::Pistol));
    }
}
