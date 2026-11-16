import type { ServerSummary } from '../protocol/ServerSummary';
import type { SessionDescription } from '../protocol/SessionDescription';

/** Lists the gameservers registered with the lobby. */
export async function fetchServers(signal?: AbortSignal): Promise<ServerSummary[]> {
  const response = await fetch('/api/servers', signal ? { signal } : {});
  const body = await read(response);
  if (!Array.isArray(body) || !body.every(isServerSummary)) {
    throw new Error('the lobby sent an unexpected server list');
  }
  return body;
}

/** Sends an SDP offer to a gameserver through the lobby and returns the gameserver's answer. */
export async function exchangeOffer(serverId: string, offer: string): Promise<string> {
  const request: SessionDescription = { sdp: offer };
  const response = await fetch(`/api/servers/${encodeURIComponent(serverId)}/connect`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(request),
  });
  const body = await read(response);
  if (!isRecord(body) || typeof body.sdp !== 'string') {
    throw new Error('the lobby sent an unexpected answer');
  }
  return body.sdp;
}

/** Picks the busiest server that still has room, so players end up together. */
export function pickServer(servers: readonly ServerSummary[]): ServerSummary | undefined {
  return servers
    .filter((server) => server.players < server.slots)
    .toSorted((a, b) => b.players - a.players || a.name.localeCompare(b.name))[0];
}

/** The JSON body of a successful response; otherwise throws the lobby's error message. */
async function read(response: Response): Promise<unknown> {
  const body: unknown = await response.json().catch(() => undefined);
  if (response.ok) {
    return body;
  }
  if (isRecord(body) && typeof body.error === 'string') {
    throw new Error(body.error);
  }
  throw new Error(`the lobby answered ${response.status} ${response.statusText}`.trim());
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isServerSummary(value: unknown): value is ServerSummary {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    typeof value.name === 'string' &&
    typeof value.players === 'number' &&
    typeof value.slots === 'number'
  );
}
