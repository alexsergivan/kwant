/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        // Indigo-tinted neutrals: the whole UI is built on zinc, so this retints it in one place
        zinc: {
          50: '#f7f6ff',
          100: '#ecebfa',
          200: '#d9d6f0',
          300: '#bcb7de',
          400: '#9a94c4',
          500: '#78719f',
          600: '#5a5383',
          700: '#3f3868',
          800: '#2c2552',
          900: '#1d1840',
          950: '#120e26',
        },
        // Primary accent: lavender instead of blue
        sky: {
          50: '#f3f1ff',
          100: '#e8e4ff',
          200: '#d3ccff',
          300: '#b8adff',
          400: '#9d8fff',
          500: '#8270f5',
          600: '#6c58e0',
          700: '#5845bd',
          800: '#46379a',
          900: '#3a2f7c',
          950: '#231b4d',
        },
        brand: {
          50: '#f0f9ff',
          100: '#e0f2fe',
          400: '#38bdf8',
          500: '#0ea5e9',
          600: '#0284c7',
        },
      },
      // Squarer, pixel-friendly corners
      borderRadius: {
        lg: '0.375rem',
        xl: '0.5rem',
        '2xl': '0.625rem',
        '3xl': '0.75rem',
      },
      fontFamily: {
        sans: ['Inter', '-apple-system', 'BlinkMacSystemFont', '"Segoe UI"', 'Roboto', 'sans-serif'],
        mono: ['"JetBrains Mono"', 'monospace'],
      },
    },
  },
  plugins: [],
}
