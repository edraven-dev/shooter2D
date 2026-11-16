import { Button } from '@shooter2d/sim-wasm';

/** Tank controls, as in the original game. */
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
  Space: Button.Fire,
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
    if (!(event.code in KEY_BUTTONS) || event.ctrlKey || event.metaKey || event.altKey) {
      return;
    }
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
