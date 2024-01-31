import { z } from 'zod'

import { assertNever } from './utils'

const { ZodFirstPartyTypeKind: TypeName } = z

/**
 * Given a Zod schema, returns a function that tries to convert a string (or
 * undefined!) to a valid input type for the schema.
 */
export function getPreprocessorByZodType(
  schema: z.ZodFirstPartySchemaTypes
): (arg: string | undefined) => unknown {
  const def = schema._def
  const { typeName } = def

  switch (typeName) {
    case TypeName.ZodString:
    case TypeName.ZodEnum:
    case TypeName.ZodUndefined:
      return arg => arg

    case TypeName.ZodNumber:
      return arg => {
        if (typeof arg === 'string' && /^-?\d+(\.\d+)?$/.test(arg)) {
          return Number(arg)
        }
        return arg
      }

    case TypeName.ZodBigInt:
      return arg => {
        if (typeof arg === 'string' && /^-?\d+$/.test(arg)) {
          return BigInt(arg)
        }
        return arg
      }

    // env vars that act as flags might be declared in a number of ways,
    // including simply `SOME_VALUE=` (with no RHS). the latter convention
    // doesn't seem to be in widespread use with node, though. (that's probably
    // because it results in the env var being present as the empty string,
    // which is falsy.)
    //
    // this preprocessor is kind of a hedge -- it accepts a few different
    // specific values to signify true or false. i can think of two other
    // options:
    // - coerce any value that's not `undefined` to `true` (or maybe any value
    //   that's not `undefined` or `false` or `0`, but again the complexity
    //   piles up quickly here).
    // - coerce *only* 'true' and 'false' to their respective values. this could
    //   be complemented by a custom schema called 'flag' or something else that
    //   handles a looser coercion case (for now this is easy for users to do in
    //   their own code according to their needs).
    //
    // for now, this hedge seems to work fine, but it might be worth revisiting.
    case TypeName.ZodBoolean:
      return arg => {
        if (typeof arg === 'string') {
          // eslint-disable-next-line default-case
          switch (arg) {
            case 'true':
            case 'yes':
            case '1':
              return true
            case 'false':
            case 'no':
            case '0':
              return false
          }
        }
        return arg
      }

    case TypeName.ZodArray:
    case TypeName.ZodObject:
    case TypeName.ZodTuple:
    case TypeName.ZodRecord:
    case TypeName.ZodIntersection:
      return arg => {
        // neither `undefined` nor the empty string are valid json.
        if (!arg) return arg
        // the one circumstance (so far) when i think a preprocessor should be
        // able to throw is if we're coercing to json but it's invalid -- this
        // way the error message will be more informative (rather than just
        // "expected x, got string"). in the future `getPreprocessor` could
        // maybe be refined to return a result type instead, but let's not
        // overengineer things for now.
        return JSON.parse(arg)
      }

    case TypeName.ZodEffects:
      return getPreprocessorByZodType(def.schema)

    case TypeName.ZodDefault:
      return getPreprocessorByZodType(def.innerType)

    case TypeName.ZodOptional: {
      const { innerType } = def
      const pp = getPreprocessorByZodType(innerType)
      return arg => {
        if (arg === undefined) return arg
        return pp(arg)
      }
    }

    case TypeName.ZodNullable: {
      const { innerType } = def
      const pp = getPreprocessorByZodType(innerType)
      return arg => {
        // coerce undefined to null.
        if (arg == null) return null
        return pp(arg)
      }
    }

    case TypeName.ZodDate:
      return arg => {
        // calling the 0-arity Date constructor makes a new Date with the
        // current time, which definitely isn't what we want here. but calling
        // the 1-arity Date constructor, even with `undefined`, should result in
        // "invalid date" for values that aren't parseable. we filter out
        // `undefined` anyway, though-- it makes typescript happier.
        if (arg == null) return arg
        return new Date(arg)
      }

    case TypeName.ZodLiteral:
      switch (typeof def.value) {
        case 'number':
          return getPreprocessorByZodType({
            _def: { typeName: TypeName.ZodNumber },
          } as z.ZodFirstPartySchemaTypes)
        case 'string':
          return getPreprocessorByZodType({
            _def: { typeName: TypeName.ZodString },
          } as z.ZodFirstPartySchemaTypes)
        case 'boolean':
          return getPreprocessorByZodType({
            _def: { typeName: TypeName.ZodBoolean },
          } as z.ZodFirstPartySchemaTypes)
        default:
          return arg => arg
      }

    case TypeName.ZodNull:
      return arg => {
        // coerce undefined to null.
        if (arg == null) return null
        return arg
      }

    case TypeName.ZodUnion:
    case TypeName.ZodNativeEnum:
      throw new Error(`Zod type not yet supported: "${typeName}" (PRs welcome)`)

    case TypeName.ZodAny:
    case TypeName.ZodUnknown:
      throw new Error(
        [
          `Zod type not supported: ${typeName}`,
          'You can use `z.string()` or `z.string().optional()` instead of the above type.',
          '(Environment variables are already constrained to `string | undefined`.)',
        ].join('\n')
      )

    // some of these types could maybe be supported (if only via the identity
    // function), but don't necessarily represent something meaningful as a
    // top-level schema passed to znv.
    case TypeName.ZodVoid:
    case TypeName.ZodNever:
    case TypeName.ZodLazy:
    case TypeName.ZodFunction:
    case TypeName.ZodPromise:
    case TypeName.ZodMap:
    case TypeName.ZodSet:
    case TypeName.ZodBranded:
    case TypeName.ZodCatch:
    case TypeName.ZodDiscriminatedUnion:
    case TypeName.ZodNaN:
    case TypeName.ZodPipeline:
      throw new Error(
        `Zod type not supported: ${typeName}. Add the implementation in @md/common/env`
      )

    default: {
      assertNever(typeName)
    }
  }
}

/**
 * Given a Zod schema, return the schema wrapped in a preprocessor that tries to
 * convert a string to the schema's input type.
 */
export function getSchemaWithPreprocessor(schema: z.ZodTypeAny) {
  return z.preprocess(getPreprocessorByZodType(schema) as (arg: unknown) => unknown, schema)
}
