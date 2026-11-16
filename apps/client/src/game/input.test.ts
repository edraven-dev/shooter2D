import { Button } from '@shooter2d/sim-wasm';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Keyboard, MOVE_BUTTONS, buttonsFor } from './input';

describe('buttonsFor', () => {
  it('maps WASD and the arrow keys to the same buttons', () => {
    expect(buttonsFor(['KeyW', 'KeyA'])).toBe(Button.Forward | Button.TurnLeft);
    expect(buttonsFor(['ArrowUp', 'ArrowLeft'])).toBe(Button.Forward | Button.TurnLeft);
  });

  it('maps the action keys', () => {
    expect(buttonsFor(['Space', 'KeyR', 'Digit2'])).toBe(Button.Fire | Button.Reload | Button.SelectPistol);
  });

  it('ignores other keys', () => {
    expect(buttonsFor(['KeyZ', 'Escape'])).toBe(0);
  });

  it('counts strafing but not turning as moving', () => {
    expect(buttonsFor(['KeyQ']) & MOVE_BUTTONS).not.toBe(0);
    expect(buttonsFor(['KeyA']) & MOVE_BUTTONS).toBe(0);
  });
});

describe('Keyboard', () => {
  type Modifiers = { ctrlKey?: boolean; altKey?: boolean; metaKey?: boolean };
  let target: EventTarget;
  let keyboard: Keyboard;

  // Stand-ins for browser KeyboardEvents: the fields the handler reads, nothing more.
  const event = (type: 'keydown' | 'keyup', code: string, modifiers: Modifiers) =>
    Object.assign(new Event(type, { cancelable: true }), {
      code,
      ctrlKey: false,
      altKey: false,
      metaKey: false,
      ...modifiers,
    });
  const press = (code: string, modifiers: Modifiers = {}) => {
    const keydown = event('keydown', code, modifiers);
    target.dispatchEvent(keydown);
    return keydown;
  };
  const release = (code: string) => target.dispatchEvent(event('keyup', code, {}));

  beforeEach(() => {
    target = new EventTarget();
    vi.stubGlobal('window', target);
    keyboard = new Keyboard();
    keyboard.attach();
  });

  afterEach(() => {
    keyboard.detach();
    vi.unstubAllGlobals();
  });

  it('fires with Space and keeps it from scrolling the page', () => {
    expect(press('Space').defaultPrevented).toBe(true);
    expect(keyboard.sample()).toBe(Button.Fire);
  });

  it('combines the keys held together', () => {
    press('Space');
    press('KeyW');
    press('KeyE');
    expect(keyboard.sample()).toBe(Button.Fire | Button.Forward | Button.StrafeRight);
  });

  it('leaves Ctrl, Alt and Meta shortcuts to the browser', () => {
    expect(press('KeyR', { ctrlKey: true }).defaultPrevented).toBe(false);
    expect(press('KeyW', { altKey: true }).defaultPrevented).toBe(false);
    expect(press('KeyD', { metaKey: true }).defaultPrevented).toBe(false);
    expect(press('ControlLeft', { ctrlKey: true }).defaultPrevented).toBe(false);
    expect(keyboard.sample()).toBe(0);
  });

  it('leaves other keys to the browser', () => {
    expect(press('KeyL').defaultPrevented).toBe(false);
    expect(keyboard.sample()).toBe(0);
  });

  it('stops on release, but counts a tap shorter than a tick once', () => {
    press('Space');
    release('Space');
    expect(keyboard.sample()).toBe(Button.Fire);
    expect(keyboard.sample()).toBe(0);
  });

  it('releases every key when the window loses focus', () => {
    press('KeyW');
    target.dispatchEvent(new Event('blur'));
    expect(keyboard.sample()).toBe(0);
  });
});
