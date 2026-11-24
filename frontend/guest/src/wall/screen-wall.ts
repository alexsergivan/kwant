// The lazy Wall big-screen chunk (pa14): only a TV whose pocket kit is `wall` loads it. It shares the quiz screen's
// stage CSS and QR maker (the screen-kit chunk) and adds its own CSS; nothing here counts toward the phone budget.
import '../screen/screen.css';
import './screen-wall.css';
export { WallTV } from './tv';
