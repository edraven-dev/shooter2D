import type { PlayerState } from '../protocol/PlayerState';
import { h } from '../ui/dom';

const KILL_FEED_SIZE = 5;
const KILL_FEED_MS = 6000;

/**
 * The heads-up display. Stats sit in a bar under the arena so they never cover players; only
 * the death notice and the kill feed are drawn over the game.
 */
export class Hud {
  /** Goes over the game canvas. */
  readonly overlay: HTMLElement;
  /** Goes under the game canvas. */
  readonly bar: HTMLElement;
  readonly #name = h('span', { className: 'hud-name' });
  readonly #score = h('span', { className: 'muted' });
  readonly #health = h('span');
  readonly #ammo = h('span');
  readonly #weapon = h('span', { className: 'muted' });
  readonly #status = h('div', { className: 'hud-status' });
  readonly #feed = h('ul', { className: 'hud-feed' });

  constructor() {
    this.overlay = h('div', { className: 'hud' }, this.#status, this.#feed);
    this.bar = h('div', { className: 'hud-bar' }, this.#name, this.#score, this.#health, this.#ammo, this.#weapon);
  }

  update(name: string, me: PlayerState): void {
    setText(this.#name, name);
    setText(this.#score, `${me.kills} kills · ${me.deaths} deaths`);
    setText(this.#health, `HP ${me.health}`);
    setText(this.#ammo, me.reloading ? 'Reloading…' : `Ammo ${me.mag} / ${me.reserve}`);
    setText(this.#weapon, me.weapon === 'rifle' ? 'Rifle' : 'Pistol');
    setText(this.#status, me.alive ? '' : 'You died. Respawning…');
  }

  addKill(killer: string, victim: string, mine: boolean): void {
    const text = killer === victim ? `${victim} died` : `${killer} ⟶ ${victim}`;
    const entry = h('li', { className: mine ? 'mine' : '' }, text);
    this.#feed.prepend(entry);
    while (this.#feed.children.length > KILL_FEED_SIZE) {
      this.#feed.lastElementChild?.remove();
    }
    setTimeout(() => entry.remove(), KILL_FEED_MS);
  }
}

function setText(element: HTMLElement, text: string): void {
  if (element.textContent !== text) {
    element.textContent = text;
  }
}
