import './style.css';

import initSimulation from '@shooter2d/sim-wasm';

import { App } from './app';

await initSimulation();

const root = document.querySelector<HTMLElement>('#app');
if (!root) {
  throw new Error('index.html has no #app element');
}
new App(root).showMenu();
