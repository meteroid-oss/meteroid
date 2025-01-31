import { parseEnv } from '@md/common'
import { z } from 'zod'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const window = globalThis as any

if (!window._env) {
  window._env = import.meta.env
}

const _env = parseEnv(window._env, {
  VITE_METEROID_API_EXTERNAL_URL: z.string().default('http://127.0.0.1:50061'),
})

export const env = {
  meteroidApiUri: _env.VITE_METEROID_API_EXTERNAL_URL,
}
