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
          // Lifted from #78719f: that was 4.2:1 on the page, so small grey text failed AA. Now 5.6:1.
          500: '#8d87b8',
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
        // "You have the wheel" in a bot's browser (Mira's stage spec §1): never sky-* (violet here), never amber
        control: {
          DEFAULT: '#7cc4ff',
          ink: '#071629',
        },
        brand: {
          50: '#f0f9ff',
          100: '#e0f2fe',
          400: '#38bdf8',
          500: '#0ea5e9',
          600: '#0284c7',
        },
      },
      // Layered surfaces, dark to light: rail < side < main < raised < overlay. Use these instead of
      // picking a zinc shade, so depth reads the same everywhere. Backed by CSS variables (modern.css).
      backgroundColor: {
        'surface-rail': 'var(--kw-surface-rail)',
        'surface-side': 'var(--kw-surface-side)',
        'surface-main': 'var(--kw-surface-main)',
        'surface-raised': 'var(--kw-surface-raised)',
        'surface-overlay': 'var(--kw-surface-overlay)',
      },
      borderColor: {
        hairline: 'var(--kw-hairline)',
        'hairline-strong': 'var(--kw-hairline-strong)',
      },
      // Status colours, one meaning each: ok = healthy/kept; working = #9d8fff, warn = needs you, bad = failed, info = neutral news.
      textColor: {
        ok: 'var(--kw-ok)',
        warn: 'var(--kw-warn)',
        bad: 'var(--kw-bad)',
        info: 'var(--kw-info)',
        muted: 'var(--kw-muted)',
      },
      backgroundImage: {
        // The one accent gradient: primary buttons, the active item, the bot-home hero glow
        'accent-gradient': 'linear-gradient(135deg, #7a62f0 0%, #5b47d6 100%)',
        'accent-soft': 'linear-gradient(135deg, rgb(122 98 240 / 0.22) 0%, rgb(91 71 214 / 0.10) 100%)',
      },
      boxShadow: {
        card: '0 1px 0 0 rgb(255 255 255 / 0.04) inset, 0 1px 2px 0 rgb(6 3 20 / 0.5)',
        raised: '0 1px 0 0 rgb(255 255 255 / 0.05) inset, 0 6px 18px -6px rgb(6 3 20 / 0.7)',
        glow: '0 0 0 1px rgb(157 143 255 / 0.35), 0 8px 24px -8px rgb(122 98 240 / 0.55)',
      },
      // Softer corners: buttons 8-10px, cards 12px, panels 16px (chips use rounded-full)
      borderRadius: {
        lg: '0.5rem',
        xl: '0.625rem',
        '2xl': '0.75rem',
        '3xl': '1rem',
      },
      // A named type scale. `text-2xs` is the smallest allowed (badges only); body copy is 14px.
      fontSize: {
        '2xs': ['0.6875rem', { lineHeight: '1rem' }],
        meta: ['0.75rem', { lineHeight: '1.125rem' }],
        body: ['0.875rem', { lineHeight: '1.375rem' }],
        title: ['1.0625rem', { lineHeight: '1.5rem', letterSpacing: '-0.01em' }],
        display: ['1.5rem', { lineHeight: '1.875rem', letterSpacing: '-0.02em' }],
      },
      fontFamily: {
        sans: ['Inter', '-apple-system', 'BlinkMacSystemFont', '"Segoe UI"', 'Roboto', 'sans-serif'],
        mono: ['"JetBrains Mono"', 'monospace'],
      },
    },
  },
  plugins: [],
}
