// Starts everything for local development: the lobby, one gameserver and the Vite dev server.
// Run it with `pnpm dev`, then open http://localhost:8080 (in two windows to play yourself).

import { type ChildProcess, spawn, spawnSync } from 'node:child_process';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { styleText } from 'node:util';

const root = path.resolve(import.meta.dirname, '..');
const exe = process.platform === 'win32' ? '.exe' : '';
const children: ChildProcess[] = [];
let stopping = false;

/** Runs a build step in the foreground and stops on failure. */
function build(command: string, args: string[]): void {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', shell: process.platform === 'win32' });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

/** Starts a long-running process, prefixing its output with its name. */
function start(name: string, color: 'cyan' | 'magenta' | 'green', command: string, args: string[]): void {
  const prefix = styleText(color, name.padEnd(10));
  const child = spawn(command, args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  for (const stream of [child.stdout, child.stderr]) {
    createInterface({ input: stream }).on('line', (line) => console.log(`${prefix} ${line}`));
  }
  child.on('exit', (code, signal) => {
    console.log(`${prefix} exited (${signal ?? code})`);
    stop();
  });
  children.push(child);
}

function stop(): void {
  if (!stopping) {
    stopping = true;
    for (const child of children) {
      child.kill('SIGINT');
    }
  }
}

process.on('SIGINT', stop);
process.on('SIGTERM', stop);

build('cargo', ['build', '-p', 'lobby', '-p', 'gameserver']);
build('pnpm', ['--filter', '@shooter2d/sim-wasm', 'run', 'build']);

start('lobby', 'cyan', path.join(root, `target/debug/lobby${exe}`), []);
start('gameserver', 'magenta', path.join(root, `target/debug/gameserver${exe}`), ['--name', 'Local']);
start('client', 'green', process.execPath, [
  path.join(root, 'apps/client/node_modules/vite/bin/vite.js'),
  'apps/client',
]);
