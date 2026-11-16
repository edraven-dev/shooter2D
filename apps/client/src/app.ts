import { Game, handshake } from './game/game';
import { Renderer } from './game/renderer';
import { loadName, randomName, saveName } from './names';
import { type GameConnection, connect } from './net/connection';
import { fetchServers, pickServer } from './net/lobby';
import type { ServerSummary } from './protocol/ServerSummary';
import { errorMessage, h } from './ui/dom';

const MAX_NAME_LENGTH = 16;
const SERVER_LIST_REFRESH_MS = 3000;

/** Switches between the menu, the server list and a running game. */
export class App {
  readonly #root: HTMLElement;
  #name = loadName();
  /** Tears down whatever the current screen started (timers, connections). */
  #cleanup: () => void = () => {};

  constructor(root: HTMLElement) {
    this.#root = root;
  }

  showMenu(notice?: string): void {
    const input = h('input', {
      value: this.#name,
      maxLength: MAX_NAME_LENGTH,
      placeholder: 'Your name',
      ariaLabel: 'Your name',
      spellcheck: false,
    });
    const remember = (): void => {
      this.#name = input.value.trim() || randomName();
      saveName(this.#name);
    };
    this.#show(
      h(
        'section',
        { className: 'menu' },
        h('h1', {}, 'shooter2D'),
        h('label', { className: 'field' }, 'Name', input),
        h('button', { className: 'primary', onclick: () => (remember(), void this.#quickPlay()) }, 'Play'),
        h('button', { onclick: () => (remember(), this.showServers()) }, 'Server list'),
        notice && h('p', { className: 'notice', role: 'alert' }, notice),
        h('p', { className: 'help' }, 'W/S move · A/D turn · Q/E strafe · Space fire · R reload · 1/2 weapons'),
      ),
    );
    input.focus();
  }

  showServers(): void {
    const list = h('ul', { className: 'servers' }, h('li', { className: 'muted' }, 'Loading…'));
    const controller = new AbortController();
    const refresh = async (): Promise<void> => {
      try {
        const servers = await fetchServers(controller.signal);
        list.replaceChildren(
          ...(servers.length > 0
            ? servers.map((server) => this.#serverRow(server))
            : [h('li', { className: 'muted' }, 'No servers are online.')]),
        );
      } catch (error) {
        if (!controller.signal.aborted) {
          list.replaceChildren(h('li', { className: 'notice' }, `Could not load servers: ${errorMessage(error)}`));
        }
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), SERVER_LIST_REFRESH_MS);
    this.#show(
      h(
        'section',
        { className: 'menu wide' },
        h('h2', {}, 'Servers'),
        list,
        h('button', { onclick: () => this.showMenu() }, 'Back'),
      ),
      () => {
        controller.abort();
        clearInterval(timer);
      },
    );
  }

  #serverRow(server: ServerSummary): HTMLLIElement {
    const full = server.players >= server.slots;
    return h(
      'li',
      {},
      h('span', { className: 'server-name' }, server.name),
      h('span', { className: 'muted' }, `${server.players} / ${server.slots}`),
      h('button', { disabled: full, onclick: () => void this.#join(server) }, full ? 'Full' : 'Join'),
    );
  }

  /** Joins the busiest server with room left. */
  async #quickPlay(): Promise<void> {
    this.#show(h('section', { className: 'menu' }, h('p', {}, 'Looking for a game…')));
    try {
      const server = pickServer(await fetchServers());
      if (server) {
        await this.#join(server);
      } else {
        this.showMenu('No server has a free slot right now.');
      }
    } catch (error) {
      this.showMenu(`Could not reach the lobby: ${errorMessage(error)}`);
    }
  }

  async #join(server: ServerSummary): Promise<void> {
    const canvasHost = h('div', { className: 'canvas-host' });
    const status = h('p', { className: 'muted' }, `Connecting to ${server.name}…`);
    const hudSlot = h('div', { className: 'hud-slot' });
    let active = true;
    let renderer: Renderer | undefined;
    let connection: GameConnection | undefined;
    let game: Game | undefined;
    // Releases whatever exists so far, at most once each (Pixi must not be destroyed twice).
    const release = (): void => {
      if (game) {
        game.stop();
      } else {
        connection?.close();
        renderer?.destroy();
      }
      game = undefined;
      connection = undefined;
      renderer = undefined;
    };
    const screen = h(
      'section',
      { className: 'game' },
      canvasHost,
      h('div', { className: 'toolbar' }, hudSlot, status, h('button', { onclick: () => this.showMenu() }, 'Leave')),
    );
    // Leaving this screen (Leave button, game over, an error) releases everything.
    this.#show(screen, () => {
      active = false;
      release();
    });

    try {
      renderer = await Renderer.create(canvasHost);
      if (active) {
        connection = await connect(server.id);
      }
      if (active && connection) {
        const welcome = await handshake(connection, this.#name);
        if (active && renderer) {
          game = new Game({ connection, renderer, welcome, onEnd: (reason) => this.showMenu(reason) });
          canvasHost.append(game.hud.overlay);
          hudSlot.append(game.hud.bar);
          status.textContent = server.name;
        }
      }
    } catch (error) {
      if (active) {
        this.showMenu(`Could not join ${server.name}: ${errorMessage(error)}`);
      }
    }
    if (!active) {
      // The player left while we were still connecting.
      release();
    }
  }

  #show(screen: HTMLElement, cleanup: () => void = () => {}): void {
    const previous = this.#cleanup;
    this.#cleanup = cleanup;
    previous();
    this.#root.replaceChildren(screen);
  }
}
