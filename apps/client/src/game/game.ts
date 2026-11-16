import { Predictor, inputRedundancy } from '@shooter2d/sim-wasm';

import { decodeServerMessage, encodeHello, encodeInputs } from '../net/codec';
import type { GameConnection } from '../net/connection';
import { InputHistory } from '../net/inputs';
import type { PlayerState } from '../protocol/PlayerState';
import type { ServerMessage } from '../protocol/ServerMessage';
import type { Snapshot } from '../protocol/Snapshot';
import type { Welcome } from '../protocol/Welcome';
import { Hud } from './hud';
import { Keyboard, MOVE_BUTTONS } from './input';
import { INTERPOLATION_DELAY_TICKS, ServerClock, SnapshotBuffer } from './interpolation';
import type { RenderedPlayer, Renderer } from './renderer';

const HANDSHAKE_TIMEOUT_MS = 5000;
/** After a stall (e.g. a background tab), catch up at most this many ticks at once. */
const MAX_TICKS_PER_FRAME = 5;

/** Says hello on the control channel and waits for the server to welcome or reject us. */
export function handshake(connection: GameConnection, name: string): Promise<Welcome> {
  return new Promise((resolve, reject) => {
    const { control } = connection;
    const finish = (settle: () => void): void => {
      clearTimeout(timer);
      control.removeEventListener('message', onMessage);
      control.removeEventListener('close', onClose);
      settle();
    };
    const onMessage = (event: MessageEvent<ArrayBuffer>): void => {
      const message = decodeServerMessage(event.data);
      if ('Welcome' in message) {
        finish(() => resolve(message.Welcome));
      } else if ('Rejected' in message) {
        finish(() => reject(new Error(message.Rejected.reason)));
      }
    };
    const onClose = (): void => finish(() => reject(new Error('the server closed the connection')));
    const timer = setTimeout(() => finish(() => reject(new Error('the server did not respond'))), HANDSHAKE_TIMEOUT_MS);
    control.addEventListener('message', onMessage);
    control.addEventListener('close', onClose);
    control.send(encodeHello(name));
  });
}

export interface GameOptions {
  connection: GameConnection;
  renderer: Renderer;
  welcome: Welcome;
  /** Called once when the game ends without {@link Game.stop}, with the reason. */
  onEnd: (reason: string) => void;
}

/**
 * A running match. Every tick (at the server's tick rate) it samples the keyboard, sends the
 * input with the previous few for redundancy, and predicts the local player with the
 * server's own movement code. Every snapshot resets that prediction to the server's state and
 * replays the inputs the server has not applied yet. Other players are drawn a little in the
 * past, interpolated between snapshots.
 */
export class Game {
  readonly hud = new Hud();
  readonly #connection: GameConnection;
  readonly #renderer: Renderer;
  readonly #onEnd: (reason: string) => void;
  readonly #playerId: number;
  readonly #tickMs: number;
  readonly #names = new Map<number, string>();
  readonly #keyboard = new Keyboard();
  readonly #inputs = new InputHistory();
  readonly #predictor = new Predictor();
  readonly #snapshots = new SnapshotBuffer();
  readonly #clock: ServerClock;
  /** The local player's latest authoritative state. */
  #me: PlayerState | undefined;
  #newestTick = -1;
  #lastButtons = 0;
  #accumulator = 0;
  #lastFrame = performance.now();
  #frame = 0;
  #stopped = false;

  constructor({ connection, renderer, welcome, onEnd }: GameOptions) {
    this.#connection = connection;
    this.#renderer = renderer;
    this.#onEnd = onEnd;
    this.#playerId = welcome.playerId;
    this.#tickMs = 1000 / welcome.tickRate;
    this.#clock = new ServerClock(welcome.tickRate);
    for (const player of welcome.players) {
      this.#names.set(player.id, player.name);
    }

    connection.control.addEventListener('message', this.#onMessage);
    connection.state.addEventListener('message', this.#onMessage);
    connection.onLost = () => this.#end('Lost the connection to the server.');
    this.#keyboard.attach();
    this.#frame = requestAnimationFrame(this.#loop);
  }

  /** Leaves the game. */
  stop(): void {
    if (this.#stopped) {
      return;
    }
    this.#stopped = true;
    cancelAnimationFrame(this.#frame);
    this.#keyboard.detach();
    this.#connection.control.removeEventListener('message', this.#onMessage);
    this.#connection.state.removeEventListener('message', this.#onMessage);
    this.#connection.close();
    this.#renderer.destroy();
    // Messages already queued may still be dispatched; #onMessage ignores them from now on.
    this.#predictor.free();
  }

  #end(reason: string): void {
    if (!this.#stopped) {
      this.stop();
      this.#onEnd(reason);
    }
  }

  readonly #loop = (now: number): void => {
    if (this.#stopped) {
      return;
    }
    this.#accumulator += Math.min(now - this.#lastFrame, this.#tickMs * MAX_TICKS_PER_FRAME);
    this.#lastFrame = now;
    while (this.#accumulator >= this.#tickMs) {
      this.#tick();
      this.#accumulator -= this.#tickMs;
    }
    this.#draw(now);
    this.#frame = requestAnimationFrame(this.#loop);
  };

  #tick(): void {
    const buttons = this.#keyboard.sample();
    this.#lastButtons = buttons;
    this.#inputs.push(buttons);
    const { firstSeq, buttons: recent } = this.#inputs.recent(inputRedundancy());
    if (this.#connection.state.readyState === 'open') {
      this.#connection.state.send(encodeInputs(firstSeq, recent));
    }
    if (this.#me?.alive) {
      this.#predictor.step(buttons);
    }
  }

  readonly #onMessage = (event: MessageEvent<ArrayBuffer>): void => {
    if (this.#stopped) {
      return;
    }
    let message: ServerMessage;
    try {
      message = decodeServerMessage(event.data);
    } catch (error) {
      console.warn('Ignoring a message that does not decode', error);
      return;
    }
    if ('Snapshot' in message) {
      this.#onSnapshot(message.Snapshot, performance.now());
    } else if ('PlayerJoined' in message) {
      this.#names.set(message.PlayerJoined.id, message.PlayerJoined.name);
    } else if ('PlayerLeft' in message) {
      this.#names.delete(message.PlayerLeft.id);
    } else if ('Kill' in message) {
      const { killer, victim } = message.Kill;
      const mine = killer === this.#playerId || victim === this.#playerId;
      this.hud.addKill(this.#name(killer), this.#name(victim), mine);
    } else if ('Rejected' in message) {
      this.#end(message.Rejected.reason);
    }
  };

  #onSnapshot(snapshot: Snapshot, now: number): void {
    this.#clock.observe(snapshot.tick, now);
    this.#snapshots.push(snapshot);
    if (snapshot.tick <= this.#newestTick) {
      return; // Arrived late: good for interpolation only.
    }
    this.#newestTick = snapshot.tick;
    const me = snapshot.players.find((player) => player.id === this.#playerId);
    if (!me) {
      return;
    }
    this.#me = me;
    this.#inputs.acknowledge(snapshot.ackSeq);
    this.#predictor.reset(me.x, me.y, me.rotation);
    if (me.alive) {
      for (const buttons of this.#inputs.pending()) {
        this.#predictor.step(buttons);
      }
    }
  }

  #draw(now: number): void {
    const serverTick = this.#clock.tick(now);
    const world = serverTick === undefined ? undefined : this.#snapshots.sample(serverTick - INTERPOLATION_DELAY_TICKS);
    const players: RenderedPlayer[] = [];
    for (const player of world?.players ?? []) {
      if (player.id !== this.#playerId) {
        players.push({ ...player, name: this.#name(player.id), local: false });
      }
    }
    const me = this.#me;
    if (me) {
      players.push({
        ...me,
        x: this.#predictor.x,
        y: this.#predictor.y,
        rotation: this.#predictor.rotation,
        moving: me.alive && (this.#lastButtons & MOVE_BUTTONS) !== 0,
        name: this.#name(me.id),
        local: true,
      });
      this.hud.update(this.#name(me.id), me);
    }
    this.#renderer.draw(players, world?.bullets ?? []);
  }

  #name(id: number): string {
    return this.#names.get(id) ?? `Player ${id}`;
  }
}
