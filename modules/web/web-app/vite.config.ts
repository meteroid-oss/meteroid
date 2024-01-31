import basicSsl from '@vitejs/plugin-basic-ssl'
import react from '@vitejs/plugin-react'
import { UserConfigExport, defineConfig } from 'vite'
import svgr from 'vite-plugin-svgr'
import tsconfigPaths from 'vite-tsconfig-paths'

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const localSsl = mode === 'dev-ssl'

  const standard: UserConfigExport = {
    server: {
      host: '0.0.0.0',
      port: 5173,
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
