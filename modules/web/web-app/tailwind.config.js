// module.exports = config({
//   content: [
//     './features/**/*.{js,ts,jsx,tsx}',
//     './pages/**/*.{js,ts,jsx,tsx}',
//     './components/**/*.{js,ts,jsx,tsx}',
//     './router/**/*.{js,ts,jsx,tsx}',
//     './lib/**/*.{js,ts,jsx,tsx}',
//     '../packages/ui/src/**/**/*.{ts,tsx}',
//     '../packages/config/radix-colors.js',
//   ],
//   theme: {
//     borderColor: theme => ({
//       ...theme('colors'),
//       DEFAULT: 'var(--colors-neutral5)',
//       dark: 'var(--colors-neutral4)',
//     }),
//     divideColor: theme => ({
//       ...theme('colors'),
//       DEFAULT: 'var(--colors-neutral6)',
//       dark: 'var(--colors-neutral2)',
//     }),
//     fontFamily: {
//       sans: ['InterVariable', 'Inter', 'Helvetica Neue', 'Helvetica', 'Arial', 'sans-serif'],
//       mono: ['source code pro', 'Menlo', 'monospace'],
//     },
//   },
//   variants: {
//     extend: {},
//   },
//   plugins: [
//     require('@tailwindcss/typography')
//   ],
// })

/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: ['class'],
  content: [
    './features/**/*.{js,ts,jsx,tsx}',
    './pages/**/*.{js,ts,jsx,tsx}',
    './components/**/*.{js,ts,jsx,tsx}',
    './router/**/*.{js,ts,jsx,tsx}',
    './lib/**/*.{js,ts,jsx,tsx}',
    '../packages/ui/src/**/**/*.{ts,tsx}',
    '../packages/ui2/src/**/**/*.{ts,tsx}',
  ],
  prefix: '',
  theme: {
    container: {
      center: true,
      padding: '2rem',
      screens: {
        '2xl': '1400px',
      },
    },
    extend: {
      colors: {
        border: 'var(--border)',
        input: 'var(--input)',
        ring: 'var(--ring)',
        background: 'var(--background)',
        foreground: 'var(--foreground)',
        primary: {
          DEFAULT: 'var(--primary)',
          foreground: 'var(--primary-foreground)',
        },
        alternative: {
          DEFAULT: 'var(--alternative)',
          foreground: 'var(--alternative-foreground)',
        },
        secondary: {
          DEFAULT: 'var(--secondary)',
          foreground: 'var(--secondary-foreground)',
        },
        destructive: {
          DEFAULT: 'var(--destructive)',
          foreground: 'var(--destructive-foreground)',
        },
        muted: {
          DEFAULT: 'var(--muted)',
          foreground: 'var(--muted-foreground)',
        },
        accent: {
          DEFAULT: 'var(--accent)',
          foreground: 'var(--accent-foreground)',
        },
        popover: {
          DEFAULT: 'var(--popover)',
          foreground: 'var(--popover-foreground)',
        },
        card: {
          DEFAULT: 'var(--card)',
          foreground: 'var(--card-foreground)',
        },
        warning: {
          DEFAULT: 'var(--warning)',
          foreground: 'var(--warning-foreground)',
        },
        success: {
          DEFAULT: 'var(--success)',
          foreground: 'var(--success-foreground)',
        },
      },
      borderRadius: {
        lg: 'var(--radius)',
        md: 'calc(var(--radius) - 2px)',
        sm: 'calc(var(--radius) - 4px)',
      },
      keyframes: {
        'accordion-down': {
          from: { height: '0' },
          to: { height: 'var(--radix-accordion-content-height)' },
        },
        'accordion-up': {
          from: { height: 'var(--radix-accordion-content-height)' },
          to: { height: '0' },
        },
      },
      animation: {
        'accordion-down': 'accordion-down 0.2s ease-out',
        'accordion-up': 'accordion-up 0.2s ease-out',
      },
    },
  },
  plugins: [require('tailwindcss-animate'), require('@tailwindcss/typography')],
}
