import { describe, expect, it } from 'vitest';

import type { PlayerState } from '../protocol/PlayerState';
import type { Snapshot } from '../protocol/Snapshot';
import { ServerClock, SnapshotBuffer, lerpAngle } from './interpolation';

function player(overrides: Partial<PlayerState> = {}): PlayerState {
  return {
    id: 1,
    x: 0,
    y: 0,
    rotation: 0,
    health: 100,
    weapon: 'rifle',
    alive: true,
    moving: false,
    reloading: false,
    shots: 0,
    mag: 30,
    reserve: 90,
    kills: 0,
    deaths: 0,
    ...overrides,
  };
}

function snapshot(tick: number, players: PlayerState[], bullets: Snapshot['bullets'] = []): Snapshot {
  return { tick, ackSeq: 0, players, bullets };
}

describe('SnapshotBuffer', () => {
  it('interpolates players between the snapshots around a tick', () => {
    const buffer = new SnapshotBuffer();
    buffer.push(snapshot(10, [player({ x: 0, y: 0 })]));
    buffer.push(snapshot(12, [player({ x: 20, y: 10 })]));
    expect(buffer.sample(11)?.players[0]).toMatchObject({ x: 10, y: 5 });
  });

  it('orders snapshots that arrive out of order', () => {
    const buffer = new SnapshotBuffer();
    buffer.push(snapshot(14, [player({ x: 40 })]));
    buffer.push(snapshot(10, [player({ x: 0 })]));
    buffer.push(snapshot(12, [player({ x: 20 })]));
    buffer.push(snapshot(12, [player({ x: 999 })])); // duplicate tick: ignored
    expect(buffer.sample(13)?.players[0]?.x).toBe(30);
  });

  it('holds the newest snapshot instead of extrapolating', () => {
    const buffer = new SnapshotBuffer();
    buffer.push(snapshot(10, [player({ x: 0 })]));
    buffer.push(snapshot(12, [player({ x: 20 })]));
    expect(buffer.sample(50)?.players[0]?.x).toBe(20);
  });

  it('does not slide respawning players across the map', () => {
    const buffer = new SnapshotBuffer();
    buffer.push(snapshot(10, [player({ x: 0, alive: false })]));
    buffer.push(snapshot(12, [player({ x: 900, alive: true })]));
    expect(buffer.sample(10.5)?.players[0]).toMatchObject({ x: 0, alive: false });
    expect(buffer.sample(11.5)?.players[0]).toMatchObject({ x: 900, alive: true });
  });

  it('interpolates bullets that exist in both snapshots', () => {
    const buffer = new SnapshotBuffer();
    buffer.push(snapshot(10, [], [{ id: 7, x: 0, y: 0, rotation: 0 }]));
    buffer.push(
      snapshot(
        12,
        [],
        [
          { id: 7, x: 24, y: 0, rotation: 0 },
          { id: 8, x: 5, y: 5, rotation: 0 },
        ],
      ),
    );
    expect(buffer.sample(11)?.bullets).toEqual([
      { id: 7, x: 12, y: 0, rotation: 0 },
      { id: 8, x: 5, y: 5, rotation: 0 },
    ]);
  });

  it('is empty before the first snapshot', () => {
    expect(new SnapshotBuffer().sample(1)).toBeUndefined();
  });
});

describe('lerpAngle', () => {
  it('turns the short way across ±π', () => {
    const from = Math.PI - 0.1;
    const to = -Math.PI + 0.1;
    expect(lerpAngle(from, to, 0.5)).toBeCloseTo(Math.PI);
  });

  it('interpolates ordinary angles linearly', () => {
    expect(lerpAngle(0, 1, 0.25)).toBeCloseTo(0.25);
  });
});

describe('ServerClock', () => {
  it('follows the earliest arrivals and advances with local time', () => {
    const clock = new ServerClock(60);
    expect(clock.tick(0)).toBeUndefined();
    clock.observe(100, 1000); // late arrival
    clock.observe(106, 1050); // 50 ms later but 6 ticks (100 ms) ahead: less delayed
    expect(clock.tick(1050)).toBeCloseTo(106);
    expect(clock.tick(1550)).toBeCloseTo(136);
  });
});
