import * as z from 'zod'

export const port = () => z.number().int().nonnegative().lte(65535)

export const deprecate = () =>
  z.undefined({ invalid_type_error: 'This var is deprecated.' }).transform(() => undefined as never)
