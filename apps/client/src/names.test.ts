import { describe, expect, it } from 'vitest';

import { randomName } from './names';

describe('randomName', () => {
  it('joins an adjective and an animal', () => {
    expect(randomName(() => 0)).toBe('BraveOtter');
    expect(randomName(() => 0.999)).toBe('KeenHeron');
  });

  it('fits the server name limit', () => {
    for (let i = 0; i < 100; i += 1) {
      expect(randomName().length).toBeLessThanOrEqual(16);
    }
  });
});
