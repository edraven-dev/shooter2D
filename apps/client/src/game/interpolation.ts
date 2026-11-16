import type { BulletState } from '../protocol/BulletState';
import type { PlayerState } from '../protocol/PlayerState';
import type { Snapshot } from '../protocol/Snapshot';

/**
 * How far behind the newest server state other players are drawn, in ticks (100 ms at 60 Hz).
 * Snapshots arrive every 2 ticks, so this rides out a couple of lost or late ones.
 */
export const INTERPOLATION_DELAY_TICKS = 6;
/** A player who moved farther than this between two snapshots teleported (respawned). */
const TELEPORT_DISTANCE = 64;
/** Snapshots this many ticks older than the newest one are dropped. */
const HISTORY_TICKS = 120;

export interface WorldView {
  players: PlayerState[];
  bullets: BulletState[];
}

/** Recent snapshots, ordered by tick, for drawing other players smoothly between them. */
export class SnapshotBuffer {
  #snapshots: Snapshot[] = [];

  /** Adds a snapshot; snapshots can arrive out of order on the unreliable channel. */
  push(snapshot: Snapshot): void {
    const snapshots = this.#snapshots;
    let index = snapshots.length;
    while (index > 0 && snapshots[index - 1]!.tick > snapshot.tick) {
      index -= 1;
    }
    if (snapshots[index - 1]?.tick === snapshot.tick) {
      return;
    }
    snapshots.splice(index, 0, snapshot);
    const newest = snapshots.at(-1)!.tick;
    while (snapshots.length > 2 && snapshots[0]!.tick < newest - HISTORY_TICKS) {
      snapshots.shift();
    }
  }

  /** The world at a fractional server tick, interpolated between the snapshots around it. */
  sample(tick: number): WorldView | undefined {
    const snapshots = this.#snapshots;
    const next = snapshots.findIndex((snapshot) => snapshot.tick > tick);
    if (next === -1 || next === 0) {
      // Past the newest snapshot (hold it) or before the oldest (show it until time catches up).
      const snapshot = next === -1 ? snapshots.at(-1) : snapshots[0];
      return snapshot && { players: snapshot.players, bullets: snapshot.bullets };
    }
    const from = snapshots[next - 1]!;
    const to = snapshots[next]!;
    const t = (tick - from.tick) / (to.tick - from.tick);
    return {
      players: interpolatePlayers(from.players, to.players, t),
      bullets: interpolateBullets(from.bullets, to.bullets, t),
    };
  }
}

function interpolatePlayers(from: readonly PlayerState[], to: readonly PlayerState[], t: number): PlayerState[] {
  const previous = new Map(from.map((player) => [player.id, player]));
  return to.map((target) => {
    const source = previous.get(target.id);
    if (!source) {
      return target;
    }
    const teleported =
      source.alive !== target.alive || Math.hypot(target.x - source.x, target.y - source.y) > TELEPORT_DISTANCE;
    if (teleported) {
      return t < 0.5 ? source : target;
    }
    return {
      ...source,
      x: lerp(source.x, target.x, t),
      y: lerp(source.y, target.y, t),
      rotation: lerpAngle(source.rotation, target.rotation, t),
    };
  });
}

function interpolateBullets(from: readonly BulletState[], to: readonly BulletState[], t: number): BulletState[] {
  const previous = new Map(from.map((bullet) => [bullet.id, bullet]));
  return to.map((target) => {
    const source = previous.get(target.id);
    return source ? { ...target, x: lerp(source.x, target.x, t), y: lerp(source.y, target.y, t) } : target;
  });
}

export function lerp(from: number, to: number, t: number): number {
  return from + (to - from) * t;
}

/** Interpolates between two angles (radians) the short way around. */
export function lerpAngle(from: number, to: number, t: number): number {
  const turn = 2 * Math.PI;
  const delta = ((((to - from) % turn) + turn + Math.PI) % turn) - Math.PI;
  return from + delta * t;
}

/**
 * Estimates the server's current tick from snapshot arrival times. It follows the earliest
 * arrivals (the least delayed ones) and drifts slowly towards later ones.
 */
export class ServerClock {
  readonly #ticksPerMs: number;
  #offset: number | undefined;

  constructor(tickRate: number) {
    this.#ticksPerMs = tickRate / 1000;
  }

  observe(tick: number, now: number): void {
    const sample = tick - now * this.#ticksPerMs;
    if (this.#offset === undefined || sample > this.#offset) {
      this.#offset = sample;
    } else {
      this.#offset += (sample - this.#offset) * 0.01;
    }
  }

  /** The estimated server tick at local time `now`, once a snapshot has been seen. */
  tick(now: number): number | undefined {
    return this.#offset === undefined ? undefined : now * this.#ticksPerMs + this.#offset;
  }
}
