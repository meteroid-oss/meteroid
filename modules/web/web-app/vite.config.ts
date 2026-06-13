import basicSsl from '@vitejs/plugin-basic-ssl'
import react from '@vitejs/plugin-react'
import { UserConfigExport, defineConfig } from 'vite'
import svgr from 'vite-plugin-svgr'
import tsconfigPaths from 'vite-tsconfig-paths'

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const localSsl = mode === 'dev-ssl'

  // Vite's default 'modules' target includes safari14, whose destructuring bug
  // esbuild >=0.28 refuses to lower. safari15 is the first version without it.
  const target = ['es2020', 'edge88', 'firefox78', 'chrome87', 'safari15']

  const standard: UserConfigExport = {
    build: { target },
    optimizeDeps: { esbuildOptions: { target } },
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
    plugins: [react(), tsconfigPaths(), svgr()],
  }
  return localSsl
    ? {
        ...standard,
        server: { ...standard.server, host: 'local.stg.meteroid.io' },
        plugins: [...(standard.plugins ?? []), basicSsl()],
      }
    : standard
})
