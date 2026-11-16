/**
 * The inputs this client has sent and the server has not acknowledged yet.
 *
 * Every tick gets the next sequence number (starting at 1). Inputs stay here until a snapshot
 * acknowledges them; client-side prediction replays the rest on top of each snapshot.
 */
export class InputHistory {
  #lastSeq = 0;
  #acknowledged = 0;
  /** Buttons by sequence number, oldest first; `#pending[0]` has number `#acknowledged + 1`. */
  #pending: number[] = [];

  /** Records the input for a new tick and returns its sequence number. */
  push(buttons: number): number {
    this.#lastSeq += 1;
    this.#pending.push(buttons);
    return this.#lastSeq;
  }

  /** The most recent `count` inputs, oldest first, for sending with redundancy. */
  recent(count: number): { firstSeq: number; buttons: Uint16Array } {
    const buttons = this.#pending.slice(-count);
    return { firstSeq: this.#lastSeq - buttons.length + 1, buttons: Uint16Array.from(buttons) };
  }

  /** Forgets every input up to and including `seq`. */
  acknowledge(seq: number): void {
    if (seq <= this.#acknowledged) {
      return;
    }
    const drop = Math.min(seq, this.#lastSeq) - this.#acknowledged;
    this.#pending.splice(0, drop);
    this.#acknowledged += drop;
  }

  /** Inputs sent but not acknowledged yet, oldest first. */
  pending(): readonly number[] {
    return this.#pending;
  }
}
