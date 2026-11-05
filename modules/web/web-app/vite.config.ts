import path from 'node:path'

import mdx from '@mdx-js/rollup'
import basicSsl from '@vitejs/plugin-basic-ssl'
import react from '@vitejs/plugin-react'
import remarkFrontmatter from 'remark-frontmatter'
import remarkMdxFrontmatter from 'remark-mdx-frontmatter'
import { UserConfigExport, defineConfig, loadEnv, searchForWorkspaceRoot } from 'vite'
import svgr from 'vite-plugin-svgr'
import tsconfigPaths from 'vite-tsconfig-paths'

// Editions outside web-app (EE) set WEB_EDITION to their entry file and WEB_EDITION_ROOT to the
// directory holding their sources and tsconfig.json; OSS builds leave both unset.
const editionRoot = process.env.WEB_EDITION_ROOT
  ? path.resolve(process.env.WEB_EDITION_ROOT)
  : undefined
const editionEntry = process.env.WEB_EDITION
  ? path.resolve(process.env.WEB_EDITION)
  : path.resolve(import.meta.dirname, 'editions/oss.ts')

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const localSsl = mode === 'dev-ssl'

  // Vite's default 'modules' target includes safari14, whose destructuring bug
  // esbuild >=0.28 refuses to lower. safari15 is the first version without it.
  const target = ['es2020', 'edge88', 'firefox78', 'chrome87', 'safari15']

  const standard: UserConfigExport = {
    resolve: { alias: { 'virtual:edition': editionEntry } },
    build: { target },
    optimizeDeps: { esbuildOptions: { target } },
    server: {
      fs: editionRoot ? { allow: [searchForWorkspaceRoot(process.cwd()), editionRoot] } : undefined,
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
    plugins: [
      // Changelog entries are authored as MDX and compiled at build time, so no
      // markdown runtime ships to the client. Frontmatter is exposed as a named
      // `frontmatter` export consumed by the changelog loader.
      {
        enforce: 'pre',
        ...mdx({
          remarkPlugins: [remarkFrontmatter, [remarkMdxFrontmatter, { name: 'frontmatter' }]],
        }),
      },
      react({ include: /\.(jsx|js|mdx|md|tsx|ts)$/ }),
      // Crawling stops at the pnpm workspace: edition sources find their tsconfig.json lazily.
      tsconfigPaths(editionRoot ? { projectDiscovery: 'lazy' } : undefined),
      svgr(),
    ],
  }
  // `dev-ssl` serves a local certificate, for the hostname DEV_SSL_HOST (root .env) when set.
  const sslHost = localSsl
    ? loadEnv(mode, path.resolve(import.meta.dirname, '../../..'), '').DEV_SSL_HOST
    : undefined
  return localSsl
    ? {
        ...standard,
        server: { ...standard.server, host: sslHost || standard.server?.host },
        plugins: [...(standard.plugins ?? []), basicSsl()],
      }
    : standard
})
