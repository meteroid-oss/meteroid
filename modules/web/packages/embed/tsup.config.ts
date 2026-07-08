import { defineConfig } from 'tsup'

export default defineConfig([
  // ESM + CJS library builds (consumed by bundlers / the React wrapper).
  {
    entry: {
      index: 'src/index.ts',
      react: 'src/react.tsx',
    },
    format: ['esm', 'cjs'],
    dts: true,
    clean: true,
    sourcemap: true,
    treeshake: true,
    // React is an optional peer dependency — never bundle it.
    external: ['react', 'react-dom'],
  },
  // Self-contained browser global, served same-origin as `/embed.js`.
  // Exposes `window.Meteroid` and auto-mounts `data-meteroid-portal` tags.
  {
    entry: { embed: 'src/global.ts' },
    format: ['iife'],
    globalName: 'Meteroid',
    platform: 'browser',
    minify: true,
    sourcemap: true,
    // Don't wipe the ESM/CJS outputs emitted by the first config.
    clean: false,
  },
])
