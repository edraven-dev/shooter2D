import { Button } from '@shooter2d/sim-wasm';

/** Tank controls, as in the original game, firing with Left Ctrl. */
const KEY_BUTTONS: Readonly<Record<string, Button>> = {
  KeyW: Button.Forward,
  ArrowUp: Button.Forward,
  KeyS: Button.Backward,
  ArrowDown: Button.Backward,
  KeyA: Button.TurnLeft,
  ArrowLeft: Button.TurnLeft,
  KeyD: Button.TurnRight,
  ArrowRight: Button.TurnRight,
  KeyQ: Button.StrafeLeft,
  KeyE: Button.StrafeRight,
  ControlLeft: Button.Fire,
  KeyR: Button.Reload,
  Digit1: Button.SelectRifle,
  Digit2: Button.SelectPistol,
};

export const MOVE_BUTTONS = Button.Forward | Button.Backward | Button.StrafeLeft | Button.StrafeRight;

/** The buttons held for a set of `KeyboardEvent.code`s. */
export function buttonsFor(codes: Iterable<string>): number {
  let buttons = 0;
  for (const code of codes) {
    buttons |= KEY_BUTTONS[code] ?? 0;
  }
  return buttons;
}

/** Tracks the game keys while attached. */
export class Keyboard {
  readonly #held = new Set<string>();
  /** Keys pressed since the last sample, so a tap shorter than a tick still counts. */
  readonly #tapped = new Set<string>();

  attach(): void {
    window.addEventListener('keydown', this.#onKeyDown);
    window.addEventListener('keyup', this.#onKeyUp);
    window.addEventListener('blur', this.#onBlur);
  }

  detach(): void {
    window.removeEventListener('keydown', this.#onKeyDown);
    window.removeEventListener('keyup', this.#onKeyUp);
    window.removeEventListener('blur', this.#onBlur);
    this.#onBlur();
  }

  /** The buttons for the next tick. */
  sample(): number {
    const buttons = buttonsFor(this.#held) | buttonsFor(this.#tapped);
    this.#tapped.clear();
    return buttons;
  }

  readonly #onKeyDown = (event: KeyboardEvent): void => {
    // Ctrl is the fire button, so holding it must not mute the other game keys. Combinations
    // with Alt or Meta stay with the browser and the OS.
    if (!(event.code in KEY_BUTTONS) || event.altKey || event.metaKey) {
      return;
    }
    // Keeps Ctrl+R, Ctrl+D, Ctrl+1… from reloading, bookmarking or switching tabs mid-fight.
    // Ctrl+W cannot be stopped this way; the game asks before the tab closes instead.
    event.preventDefault();
    this.#held.add(event.code);
    this.#tapped.add(event.code);
  };

  readonly #onKeyUp = (event: KeyboardEvent): void => {
    this.#held.delete(event.code);
  };

  readonly #onBlur = (): void => {
    this.#held.clear();
    this.#tapped.clear();
  };
}
