import basicSsl from '@vitejs/plugin-basic-ssl'
import react from '@vitejs/plugin-react'
import { UserConfigExport, defineConfig } from 'vite'
import circleDependency from 'vite-plugin-circular-dependency'
import svgr from 'vite-plugin-svgr'
import tsconfigPaths from 'vite-tsconfig-paths'

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const localSsl = mode === 'dev-ssl'

  const standard: UserConfigExport = {
    server: {
      host: '0.0.0.0',
      port: 5173,
      proxy: {
        '/api': {
          target: 'http://localhost:8080',
          changeOrigin: true,
          secure: false,
          rewrite: path => path.replace(/^\/api/, ''),
        },
      },
    },
    envDir: '../../../',
    plugins: [react(), tsconfigPaths(), svgr(), circleDependency()],
  }
  return localSsl
    ? {
        ...standard,
        server: { ...standard.server, host: 'local.stg.meteroid.io' },
        plugins: [...(standard.plugins ?? []), basicSsl()],
      }
    : standard
})
