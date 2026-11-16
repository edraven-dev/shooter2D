import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ServerSummary } from '../protocol/ServerSummary';
import { exchangeOffer, fetchServers, pickServer } from './lobby';

const server = (id: string, players: number, slots = 8): ServerSummary => ({
  id,
  name: `Server ${id}`,
  players,
  slots,
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('pickServer', () => {
  it('prefers the busiest server with room', () => {
    expect(pickServer([server('a', 1), server('b', 5), server('c', 8)])?.id).toBe('b');
  });

  it('finds nothing when every server is full', () => {
    expect(pickServer([server('a', 8), server('b', 2, 2)])).toBeUndefined();
  });
});

describe('lobby requests', () => {
  it('posts the offer and returns the answer', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () => Response.json({ sdp: 'answer' }));
    vi.stubGlobal('fetch', fetch);
    await expect(exchangeOffer('a/b', 'offer')).resolves.toBe('answer');
    expect(fetch).toHaveBeenCalledWith('/api/servers/a%2Fb/connect', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ sdp: 'offer' }),
    });
  });

  it('surfaces the lobby error message', async () => {
    vi.stubGlobal('fetch', async () => Response.json({ error: 'the server is full' }, { status: 409 }));
    await expect(exchangeOffer('a', 'offer')).rejects.toThrow('the server is full');
  });

  it('rejects malformed server lists', async () => {
    vi.stubGlobal('fetch', async () => Response.json([{ id: 1 }]));
    await expect(fetchServers()).rejects.toThrow('unexpected server list');
  });

  it('copes with error responses that are not JSON', async () => {
    vi.stubGlobal('fetch', async () => new Response('bad gateway', { status: 502, statusText: 'Bad Gateway' }));
    await expect(fetchServers()).rejects.toThrow('the lobby answered 502 Bad Gateway');
  });
});
