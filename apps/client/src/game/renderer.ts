import { levelHeight, levelWalls, levelWidth } from '@shooter2d/sim-wasm';
import { AnimatedSprite, Application, Assets, Container, Graphics, Sprite, Text } from 'pixi.js';
import type { Spritesheet, Texture } from 'pixi.js';

import type { BulletState } from '../protocol/BulletState';
import type { PlayerState } from '../protocol/PlayerState';

const SPRITESHEET_URL = `${import.meta.env.BASE_URL}assets/spritesheet.json`;
const PLAYER_SCALE = 0.25;
const BULLET_SCALE = 0.15;
/** Sprite frames advance at half the display rate: 30 fps on a 60 Hz screen. */
const ANIMATION_SPEED = 0.5;
const HEALTH_BAR_WIDTH = 40;

/** A player as drawn: their state plus what the renderer needs to label them. */
export interface RenderedPlayer extends PlayerState {
  name: string;
  local: boolean;
}

/** Draws the arena, players and bullets with Pixi.js. */
export class Renderer {
  readonly #app: Application;
  readonly #sheet: Spritesheet;
  readonly #players = new Map<number, PlayerView>();
  readonly #bullets = new Map<number, Sprite>();
  readonly #playerLayer = new Container();
  readonly #bulletLayer = new Container();

  static async create(host: HTMLElement): Promise<Renderer> {
    const app = new Application();
    await app.init({
      width: levelWidth(),
      height: levelHeight(),
      background: '#6e6e6e',
      antialias: true,
      // Render at the screen's density; CSS scales the canvas to fit the window.
      resolution: window.devicePixelRatio,
      autoDensity: false,
    });
    const sheet = await Assets.load<Spritesheet>(SPRITESHEET_URL);
    host.append(app.canvas);
    return new Renderer(app, sheet);
  }

  private constructor(app: Application, sheet: Spritesheet) {
    this.#app = app;
    this.#sheet = sheet;
    const walls = new Graphics();
    const rects = levelWalls();
    for (let i = 0; i + 3 < rects.length; i += 4) {
      walls.rect(rects[i]!, rects[i + 1]!, rects[i + 2]!, rects[i + 3]!);
    }
    walls.fill(0x2c2f36);
    app.stage.addChild(walls, this.#bulletLayer, this.#playerLayer);
  }

  draw(players: readonly RenderedPlayer[], bullets: readonly BulletState[]): void {
    const seenPlayers = new Set<number>();
    for (const player of players) {
      seenPlayers.add(player.id);
      let view = this.#players.get(player.id);
      if (!view) {
        view = new PlayerView(this.#sheet, player);
        this.#players.set(player.id, view);
        this.#playerLayer.addChild(view.container);
      }
      view.update(player);
    }
    for (const [id, view] of this.#players) {
      if (!seenPlayers.has(id)) {
        view.container.destroy({ children: true });
        this.#players.delete(id);
      }
    }

    const seenBullets = new Set<number>();
    for (const bullet of bullets) {
      seenBullets.add(bullet.id);
      let sprite = this.#bullets.get(bullet.id);
      if (!sprite) {
        sprite = new Sprite(this.#texture('bullet.png'));
        sprite.anchor.set(0.5);
        sprite.scale.set(BULLET_SCALE);
        sprite.tint = 0x000000;
        this.#bullets.set(bullet.id, sprite);
        this.#bulletLayer.addChild(sprite);
      }
      sprite.position.set(bullet.x, bullet.y);
      sprite.rotation = bullet.rotation;
    }
    for (const [id, sprite] of this.#bullets) {
      if (!seenBullets.has(id)) {
        sprite.destroy();
        this.#bullets.delete(id);
      }
    }
  }

  destroy(): void {
    // Textures stay cached in Assets for the next game.
    this.#app.destroy({ removeView: true }, { children: true });
  }

  #texture(name: string): Texture {
    const texture = this.#sheet.textures[name];
    if (!texture) {
      throw new Error(`missing texture ${name}`);
    }
    return texture;
  }
}

class PlayerView {
  readonly container = new Container();
  readonly #sheet: Spritesheet;
  readonly #body: AnimatedSprite;
  readonly #label: Text;
  readonly #healthBar = new Graphics();
  #animation = '';
  #shots: number;
  #health = -1;
  /** Playing a one-off animation (shooting) that must finish before the next one starts. */
  #busy = false;

  constructor(sheet: Spritesheet, state: RenderedPlayer) {
    this.#sheet = sheet;
    this.#shots = state.shots;
    this.#body = new AnimatedSprite(this.#frames(`player_idle_${state.weapon}`));
    this.#body.anchor.set(0.5);
    this.#body.scale.set(PLAYER_SCALE);
    this.#body.animationSpeed = ANIMATION_SPEED;
    this.#body.onComplete = () => {
      this.#busy = false;
    };
    this.#label = new Text({
      text: state.name,
      style: { fill: state.local ? 0xffd166 : 0xffffff, fontSize: 13, fontFamily: 'system-ui, sans-serif' },
    });
    this.#label.anchor.set(0.5, 1);
    this.#label.position.set(0, -34);
    this.#healthBar.position.set(-HEALTH_BAR_WIDTH / 2, -32);
    this.container.addChild(this.#body, this.#healthBar, this.#label);
  }

  update(state: RenderedPlayer): void {
    this.container.visible = state.alive;
    this.container.position.set(state.x, state.y);
    this.#body.rotation = state.rotation;
    if (this.#label.text !== state.name) {
      this.#label.text = state.name;
    }
    if (state.health !== this.#health) {
      this.#health = state.health;
      const filled = (HEALTH_BAR_WIDTH * state.health) / 100;
      this.#healthBar
        .clear()
        .rect(0, 0, HEALTH_BAR_WIDTH, 4)
        .fill(0x1b1b1b)
        .rect(0, 0, filled, 4)
        .fill(state.health > 30 ? 0x6bd36b : 0xe55353);
    }

    if (state.shots !== this.#shots) {
      this.#shots = state.shots;
      this.#play(`player_shoot_${state.weapon}`, false);
    } else if (!this.#busy) {
      const action = state.reloading ? 'reload' : state.moving ? 'move' : 'idle';
      this.#play(`player_${action}_${state.weapon}`, true);
    }
  }

  #play(animation: string, loop: boolean): void {
    if (loop && animation === this.#animation) {
      return;
    }
    this.#animation = animation;
    this.#busy = !loop;
    this.#body.textures = this.#frames(animation);
    this.#body.loop = loop;
    this.#body.gotoAndPlay(0);
  }

  #frames(animation: string): Texture[] {
    const frames = this.#sheet.animations[animation];
    if (!frames) {
      throw new Error(`missing animation ${animation}`);
    }
    return frames;
  }
}
