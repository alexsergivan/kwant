// The lazy Queue big-screen chunk (pa11b): only a TV whose pocket kit is `queue` loads it. It shares the quiz screen's
// stage CSS and QR maker (the screen-kit chunk) and adds its own CSS; nothing here counts toward the phone budget.
import '../screen/screen.css';
import './screen-queue.css';
export { QueueTV } from './tv';
