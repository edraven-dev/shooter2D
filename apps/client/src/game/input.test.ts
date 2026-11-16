import { Button } from '@shooter2d/sim-wasm';
import { describe, expect, it } from 'vitest';

import { MOVE_BUTTONS, buttonsFor } from './input';

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
