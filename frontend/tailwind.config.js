/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        arcade: {
          dark: '#0d0c13',
          panel: '#181622',
          border: '#2c283e',
          amber: '#fca311',
          cyan: '#00f5d4',
          magenta: '#f72585',
          green: '#38b000',
          muted: '#8b8a9e',
        }
      },
      fontFamily: {
        pixel: ['"Press Start 2P"', 'monospace'],
        retro: ['"Silkscreen"', 'monospace'],
        terminal: ['"VT323"', 'monospace'],
      },
      boxShadow: {
        'pixel': '3px 3px 0px 0px #000000',
        'pixel-sm': '2px 2px 0px 0px #000000',
        'pixel-amber': '3px 3px 0px 0px #fca311',
        'pixel-cyan': '3px 3px 0px 0px #00f5d4',
      }
    },
  },
  plugins: [],
}
