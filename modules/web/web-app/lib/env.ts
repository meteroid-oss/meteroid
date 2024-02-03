import { parseEnv } from '@md/common'
import { z } from 'zod'

const _env = parseEnv(import.meta.env, {
  VITE_METEROID_API_EXTERNAL_URL: z.string().default('http://127.0.0.1:50061'),
})

export const env = {
  meteroidApiUri: _env.VITE_METEROID_API_EXTERNAL_URL,
}
