import { parseEnv } from '@md/common'
import { z } from 'zod'

const _env = parseEnv(import.meta.env, {
  VITE_DEBUG_MODE: z.boolean().default(false),
  VITE_API_URL: z.string().default('http://127.0.0.1:8000'),
  METEROID_URI: z.string().default('http://127.0.0.1:50061'),
})

export const env = {
  isDebug: _env.VITE_DEBUG_MODE,
  apiUrl: _env.VITE_API_URL,
  meteroidApiUri: _env.METEROID_URI,
}
