import { atom } from 'jotai'
import { z } from 'zod'

function getDefaults<Schema extends z.AnyZodObject>(schema: Schema) {
  return Object.fromEntries(
    Object.entries(schema.shape).map(([key, value]) => {
      if (value instanceof z.ZodDefault) return [key, value._def.defaultValue()]
      return [key, undefined]
    })
  )
}

export function zatom<TSchema extends z.AnyZodObject>(schema: TSchema) {
  const typedAtom = atom<z.infer<typeof schema> | undefined>(getDefaults(schema))
  return typedAtom
}
