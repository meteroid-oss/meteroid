import { parseEnv } from '@md/common'
import { z } from 'zod/v3'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const window = globalThis as any

if (!window._env) {
  window._env = import.meta.env
}

const _env = parseEnv(window._env, {
  VITE_METEROID_API_EXTERNAL_URL: z.string().default('http://127.0.0.1:50061'),
  VITE_METEROID_REST_API_EXTERNAL_URL: z.string().default('http://127.0.0.1:8080'),
  // enable developer experience mode
  VITE_DX: z.boolean().default(false),
  // OpenReplay
  VITE_PUBLIC_OPENREPLAY_PROJECT_KEY: z.string().optional(),
  VITE_PUBLIC_OPENREPLAY_INGEST_POINT: z.string().optional(),
})

export const env = {
  meteroidApiUri: _env.VITE_METEROID_API_EXTERNAL_URL,
  meteroidRestApiUri: _env.VITE_METEROID_REST_API_EXTERNAL_URL,
  dx: _env.VITE_DX,
  openReplayProjectKey: _env.VITE_PUBLIC_OPENREPLAY_PROJECT_KEY,
  openReplayIngestPoint: _env.VITE_PUBLIC_OPENREPLAY_INGEST_POINT,
  entitlementsEnabled: true,
}
