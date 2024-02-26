const config = require('@md/config/tailwind.config')

module.exports = config({
  content: [
    './features/**/*.{js,ts,jsx,tsx}',
    './pages/**/*.{js,ts,jsx,tsx}',
    './components/**/*.{js,ts,jsx,tsx}',
    './router/**/*.{js,ts,jsx,tsx}',
    './lib/**/*.{js,ts,jsx,tsx}',
    '../packages/ui/src/**/**/*.{ts,tsx}',
    '../packages/config/radix-colors.js',
  ],
  theme: {
    borderColor: theme => ({
      ...theme('colors'),
      DEFAULT: 'var(--colors-neutral5)',
      dark: 'var(--colors-neutral4)',
    }),
    divideColor: theme => ({
      ...theme('colors'),
      DEFAULT: 'var(--colors-neutral6)',
      dark: 'var(--colors-neutral2)',
    }),
    fontFamily: {
      sans: ['InterVariable', 'Inter', 'Helvetica Neue', 'Helvetica', 'Arial', 'sans-serif'],
      mono: ['source code pro', 'Menlo', 'monospace'],
    },
  },
  variants: {
    extend: {},
  },
  plugins: [
    require('@tailwindcss/typography')
  ],
})
