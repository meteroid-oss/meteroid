import path from 'node:path'

import { defineConfig } from 'vite'
import dts from 'vite-plugin-dts'

export default defineConfig(configEnv => ({
  plugins: [
    dts({
      insertTypesEntry: true,
    }),
  ],

  build: {
    sourcemap: true,
    lib: {
      entry: path.resolve(__dirname, 'src/index.ts'),
      name: 'foundation',
      formats: ['es', 'umd'],
      fileName: format => `index.${format}.js`,
    },
  },
}))
