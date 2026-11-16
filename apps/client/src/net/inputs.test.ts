import { describe, expect, it } from 'vitest';

import { InputHistory } from './inputs';

describe('InputHistory', () => {
  it('numbers inputs from 1', () => {
    const history = new InputHistory();
    expect(history.push(5)).toBe(1);
    expect(history.push(6)).toBe(2);
  });

  it('sends the most recent inputs with their first sequence number', () => {
    const history = new InputHistory();
    for (const buttons of [1, 2, 3, 4, 5, 6]) {
      history.push(buttons);
    }
    const { firstSeq, buttons } = history.recent(4);
    expect(firstSeq).toBe(3);
    expect([...buttons]).toEqual([3, 4, 5, 6]);
  });

  it('sends fewer inputs at the start', () => {
    const history = new InputHistory();
    history.push(9);
    expect(history.recent(4)).toEqual({ firstSeq: 1, buttons: Uint16Array.of(9) });
  });

  it('keeps unacknowledged inputs for replay', () => {
    const history = new InputHistory();
    for (const buttons of [1, 2, 3, 4]) {
      history.push(buttons);
    }
    history.acknowledge(2);
    expect(history.pending()).toEqual([3, 4]);
    history.acknowledge(1); // stale acknowledgements change nothing
    expect(history.pending()).toEqual([3, 4]);
    history.acknowledge(4);
    expect(history.pending()).toEqual([]);
  });

  it('still sends recent inputs after they are acknowledged', () => {
    const history = new InputHistory();
    for (const buttons of [1, 2, 3]) {
      history.push(buttons);
    }
    history.acknowledge(3);
    history.push(4);
    // Only the unacknowledged input is left to resend.
    expect(history.recent(4)).toEqual({ firstSeq: 4, buttons: Uint16Array.of(4) });
  });
});
