import {
  decodeServerMessage as decodeMessage,
  encodeHello as encodeHelloMessage,
  encodeInputs as encodeInputsMessage,
} from '@shooter2d/sim-wasm';

import type { ServerMessage } from '../protocol/ServerMessage';

// The Rust codec (compiled to WASM) does the work, so client and server share one codec.

/**
 * Decodes a message from the gameserver. Its shape matches the types generated from the same
 * Rust definitions.
 */
export function decodeServerMessage(data: ArrayBuffer): ServerMessage {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- postcard validated the bytes and serde built the object from the Rust type this one is generated from.
  return decodeMessage(new Uint8Array(data)) as ServerMessage;
}

export function encodeHello(name: string): Uint8Array<ArrayBuffer> {
  return plain(encodeHelloMessage(name));
}

/** Encodes the inputs numbered `firstSeq..`, oldest first. */
export function encodeInputs(firstSeq: number, buttons: Uint16Array): Uint8Array<ArrayBuffer> {
  return plain(encodeInputsMessage(firstSeq, buttons));
}

/**
 * Narrows bytes to a view over a plain `ArrayBuffer`, which `RTCDataChannel.send()` accepts.
 * wasm-bindgen already copies its output into one, so this never copies in practice.
 */
function plain(bytes: Uint8Array): Uint8Array<ArrayBuffer> {
  return bytes.buffer instanceof ArrayBuffer
    ? new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength)
    : new Uint8Array(bytes);
}
