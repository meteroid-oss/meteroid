import path from 'node:path'

import react from '@vitejs/plugin-react'
import autoprefixer from 'autoprefixer'
import { defineConfig } from 'vite'
import dts from 'vite-plugin-dts'
import tsconfigPaths from 'vite-tsconfig-paths'

export default defineConfig(() => ({
  plugins: [
    react(),
    tsconfigPaths(),
    dts({
      insertTypesEntry: true,
    }),
  ],
  css: {
    postcss: {
      plugins: [autoprefixer({})],
    },
  },
  build: {
    sourcemap: true,
    lib: {
      entry: path.resolve(__dirname, 'src/index.ts'),
      name: 'ui',
    },
    rollupOptions: {
      // allow importing external dependencies
      external: ['react-router-dom'],
      output: {
        globals: {
          'react-router-dom': 'react-router-dom',
        },
      },
    },
  },
}))
