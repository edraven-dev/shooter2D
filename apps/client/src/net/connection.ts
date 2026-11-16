import { controlChannelLabel, stateChannelLabel } from '@shooter2d/sim-wasm';

import { exchangeOffer } from './lobby';

const OPEN_TIMEOUT_MS = 10_000;

/** A WebRTC connection to a gameserver with its two data channels open. */
export interface GameConnection {
  /** Reliable and ordered: handshake, roster changes, kills. */
  readonly control: RTCDataChannel;
  /** Unreliable and unordered: inputs up, snapshots down. */
  readonly state: RTCDataChannel;
  /** Fires once when the connection drops for any reason other than {@link close}. */
  onLost?: () => void;
  close(): void;
}

/**
 * Connects to a gameserver: offers two data channels, lets the lobby relay the offer, and
 * waits for both channels to open.
 *
 * The offer goes out without waiting for ICE gathering: the gameserver is ICE-lite with a
 * public host candidate, so it learns our address from our connectivity checks.
 */
export async function connect(serverId: string): Promise<GameConnection> {
  const peer = new RTCPeerConnection();
  const control = peer.createDataChannel(controlChannelLabel());
  const state = peer.createDataChannel(stateChannelLabel(), { ordered: false, maxRetransmits: 0 });
  control.binaryType = 'arraybuffer';
  state.binaryType = 'arraybuffer';

  try {
    const offer = await peer.createOffer();
    await peer.setLocalDescription(offer);
    if (offer.sdp === undefined) {
      throw new Error('the browser produced an empty offer');
    }
    const answer = await exchangeOffer(serverId, offer.sdp);
    await peer.setRemoteDescription({ type: 'answer', sdp: answer });
    await withTimeout(Promise.all([opened(control), opened(state)]), OPEN_TIMEOUT_MS, 'the connection timed out');
  } catch (error) {
    peer.close();
    throw error;
  }

  let closed = false;
  const connection: GameConnection = {
    control,
    state,
    close() {
      closed = true;
      peer.close();
    },
  };
  const lost = (): void => {
    if (!closed) {
      closed = true;
      peer.close();
      connection.onLost?.();
    }
  };
  control.addEventListener('close', lost);
  peer.addEventListener('connectionstatechange', () => {
    if (peer.connectionState === 'failed' || peer.connectionState === 'closed') {
      lost();
    }
  });
  return connection;
}

function opened(channel: RTCDataChannel): Promise<void> {
  if (channel.readyState === 'open') {
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    channel.addEventListener('open', () => resolve(), { once: true });
    channel.addEventListener('close', () => reject(new Error(`the ${channel.label} channel closed`)), { once: true });
  });
}

function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}
