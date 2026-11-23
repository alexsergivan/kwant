// The lazy big-screen chunk (pa5b): only /p/<code>#screen=<token> loads it, so guests' phones never download it.
// Its CSS ships as its own file next to this chunk.
import './screen.css';
export { ScreenApp } from './app';
