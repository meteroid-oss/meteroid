import { mergeConfig } from 'vite'
import { defineConfig } from 'vitest/config'

import viteConfig from './vite.config'

// Reuses the app's vite config (plugins, aliases) so tests resolve `@/` the same way the app does.
export default defineConfig(configEnv =>
  mergeConfig(viteConfig(configEnv), {
    test: {
      environment: 'node',
      include: ['**/*.test.ts', '**/*.test.tsx'],
    },
  })
)
