# shooter2D

A top-down multiplayer shooter for the browser. Rust servers run the game and talk to browsers
over WebRTC data channels; the TypeScript client renders with Pixi.js and predicts its own
movement with the server's code compiled to WebAssembly.

## Architecture

```text
             HTTP (fetch)                                       WebSocket (gameserver dials out)
 browser ──▶ GET  /api/servers                    ┌───────┐
    │        POST /api/servers/{id}/connect ────▶ │ lobby │ ◀──── register, player count ────┐
    │             SDP offer ──▶ ◀── SDP answer    └───────┘ ───── relays the offer ─────▶ ┌────────────┐
    │                                                       ◀──── SDP answer ───────────  │ gameserver │
    │                                                                                     └────────────┘
    └════ WebRTC over UDP (DTLS + SCTP) ════════════════════════════════════════════════════════▶
            "control" channel: reliable, ordered      hello, roster changes, kills
            "state" channel:   unreliable, unordered  inputs up (60 Hz), snapshots down (30 Hz)
```

| Path              | What it is                                                                                        | Built with                |
| ----------------- | ------------------------------------------------------------------------------------------------- | ------------------------- |
| `apps/lobby`      | Server list and signaling relay; can also serve the built client                                  | Rust, axum, tokio         |
| `apps/gameserver` | One authoritative match: WebRTC peers on one UDP socket, 60 Hz simulation, 30 Hz snapshots        | Rust, str0m, tokio        |
| `apps/client`     | Menus (plain DOM), rendering, input, WebRTC, prediction and interpolation                         | TypeScript, Vite, Pixi.js |
| `crates/protocol` | Every wire type: lobby JSON and data-channel messages (postcard); generates the client's TS types | serde, postcard, ts-rs    |
| `crates/sim`      | Deterministic simulation: movement, weapons, bullets, respawns                                    | Rapier2D                  |
| `crates/sim-wasm` | The codec and movement prediction compiled to WebAssembly for the client                          | wasm-bindgen, wasm-pack   |

Why it is split this way:

- **Rust servers.** [str0m](https://github.com/algesten/str0m) is a sans-IO WebRTC stack: one UDP
  socket serves every peer, and the game loop drives it, with no garbage-collector pauses and a
  single static binary to deploy.
- **TypeScript client.** WebRTC, the DOM and WebGL/WebGPU are browser APIs, so the client stays in
  TypeScript. The only code worth sharing with the server is the simulation and the codec, which
  arrive as a small WASM module.
- **Gameservers dial out.** Browsers reach the lobby over HTTPS and the gameservers over UDP, so a
  gameserver needs an open UDP port but no domain, TLS certificate or HTTP endpoint.
- **Physics: Rapier2D.** Walls are Rapier colliders; players move with shape casts (move and
  slide) and bullets with ray casts. Its `enhanced-determinism` feature plus `libm` make the
  native server and the WASM client compute bit-identical movement, which the end-to-end test
  `server_movement_matches_client_prediction_bit_for_bit` checks over a real WebRTC connection.

### Netcode

- The client sends one input per tick (60 Hz), repeating the previous three so a lost packet
  never loses an input. The server applies every input exactly once, in order.
- Snapshots go out at 30 Hz. Each one tells its client the last input the server applied.
- The client predicts its own player: on every snapshot it resets to the server's state and
  replays the inputs the server has not applied yet, using the server's own movement code.
- Other players are drawn 100 ms in the past, interpolated between snapshots.

## Requirements

- **Node.js 26 and pnpm 12.** pnpm downloads the pinned Node.js on its own (`devEngines` in
  `package.json`).
- **Rust 1.100 through rustup.** `rust-toolchain.toml` installs the toolchain, clippy, rustfmt and
  the `wasm32-unknown-unknown` target on first use.
- **A C compiler and CMake**, to build `aws-lc-rs` (the crypto behind str0m's DTLS).

## Getting started

```sh
pnpm install
pnpm dev
```

`pnpm dev` builds everything, then starts the lobby (`:3000`), a gameserver (UDP `:7777`) and
Vite (`:8080`). Open <http://localhost:8080> in two windows to play against yourself.

Controls: <kbd>W</kbd>/<kbd>S</kbd> move, <kbd>A</kbd>/<kbd>D</kbd> turn, <kbd>Q</kbd>/<kbd>E</kbd>
strafe, <kbd>Left Ctrl</kbd> fire, <kbd>R</kbd> reload, <kbd>1</kbd>/<kbd>2</kbd> rifle/pistol.

## Commands

| Command               | Does                                                                            |
| --------------------- | ------------------------------------------------------------------------------- |
| `pnpm dev`            | Runs the lobby, a gameserver and the Vite dev server                            |
| `pnpm build`          | Builds the client into `apps/client/dist` and the servers into `target/release` |
| `pnpm check`          | Everything CI should run: formatting, lint, type check, tests                   |
| `pnpm fmt`            | Formats TypeScript, JSON, CSS and Markdown (oxfmt) and Rust (rustfmt)           |
| `pnpm lint`           | oxlint with type-aware rules, and clippy                                        |
| `pnpm typecheck`      | TypeScript 7 over the client and the scripts                                    |
| `pnpm test`           | Vitest, then `cargo test` (including WebRTC end-to-end tests)                   |
| `pnpm protocol:types` | Regenerates `apps/client/src/protocol` after changing `crates/protocol`         |

## Configuration

Both servers take flags (`--help`) or environment variables:

| Lobby              | Default          | Meaning                                                     |
| ------------------ | ---------------- | ----------------------------------------------------------- |
| `LOBBY_LISTEN`     | `127.0.0.1:3000` | HTTP and WebSocket address                                  |
| `GAMESERVER_TOKEN` | unset            | Secret gameservers must present; set it in production       |
| `STATIC_DIR`       | unset            | Serves the built client (`apps/client/dist`) on other paths |

| Gameserver         | Default                               | Meaning                                                     |
| ------------------ | ------------------------------------- | ----------------------------------------------------------- |
| `GAMESERVER_NAME`  | `shooter2d`                           | Name in the server list                                     |
| `SLOTS`            | `8`                                   | Most players at once                                        |
| `LOBBY_URL`        | `ws://127.0.0.1:3000/api/gameservers` | Lobby WebSocket                                             |
| `GAMESERVER_TOKEN` | unset                                 | Must match the lobby's                                      |
| `BIND`             | `0.0.0.0:7777`                        | UDP address for WebRTC                                      |
| `PUBLIC_IP`        | IP of the default route               | Address browsers send to; set it behind 1:1 NAT (cloud VMs) |

## Deploying

- **Lobby:** put it behind HTTPS with `GAMESERVER_TOKEN` set, and serve the client from the same
  origin (`STATIC_DIR`, or a reverse proxy in front of both), because the client calls `/api` on
  its own origin. Gameservers then connect with `wss://`.
- **Gameservers:** open the UDP port and set `PUBLIC_IP` when the machine's own address is not the
  public one. One process runs one match; run more on other ports. Browsers silently refuse some
  ports (6665–6669, for example), so the gameserver refuses to start on them.
- No TURN server is needed while gameservers have public IPs. Players whose network blocks UDP
  cannot connect; a TURN relay would fix that.

## Contributing

- **Git flow:** `master` holds releases and `develop` integrates; branch `feature/*` from
  `develop`, and use `release/*` and `hotfix/*` as usual.
- **[Conventional Commits](https://www.conventionalcommits.org/):** `feat(client): …`,
  `fix(gameserver): …`, `build: …`, with `!` for breaking changes.
- Run `pnpm check` before pushing.
