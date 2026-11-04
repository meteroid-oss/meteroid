import { useCallback, useEffect, useMemo, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

export type SetQueryStateAction<T> = T | ((prevState: T) => T)

export const ARRAY_SERDE = {
  serialize: (value: string[]) => value.join(','),
  deserialize: (value: string) => value.split(','),
}

// URL-friendly serde for a protobuf enum, storing the readable enum name
// (e.g. ?status=FINALIZED) and mapping undefined to an absent param.
export const enumSerde = <E extends Record<string, string | number>>(enumObj: E) => ({
  serialize: (value: number | undefined) =>
    value === undefined ? '' : String(enumObj[value as keyof E] ?? ''),
  deserialize: (value: string) => {
    const parsed = enumObj[value as keyof E]
    return typeof parsed === 'number' ? parsed : undefined
  },
})

export function useQueryState<T>(
  key: string,
  defaultValue: T,
  {
    serialize = (value: T) => (typeof value === 'string' ? value : JSON.stringify(value)),
    deserialize = (value: string) => {
      try {
        return JSON.parse(value)
      } catch {
        return value
      }
    },
    replace = false,
  }: {
    serialize?: (value: T) => string
    deserialize?: (value: string) => T
    /** Replace the history entry instead of pushing one (keeps Back out of the state changes). */
    replace?: boolean
  } = {}
): [T, (value: SetQueryStateAction<T>) => void] {
  const [searchParams, setSearchParams] = useSearchParams()
  const paramValue = searchParams.get(key)
  const state = useMemo(
    () => (paramValue === null ? defaultValue : deserialize(paramValue)),
    [paramValue, defaultValue, deserialize]
  )

  const setQueryState = useCallback(
    (value: SetQueryStateAction<T>) => {
      const newState = typeof value === 'function' ? (value as (prevState: T) => T)(state) : value

      setSearchParams(
        prevParams => {
          const newParams = new URLSearchParams(prevParams)
          const serialized = serialize(newState)

          if (newState === defaultValue || serialized === '') {
            newParams.delete(key)
          } else {
            newParams.set(key, serialized)
          }
          return newParams
        },
        { replace }
      )
    },
    [key, setSearchParams, serialize, defaultValue, state, replace]
  )

  return [state, setQueryState]
}

export function useQueryRecordState<T extends Record<string, string | number>>(
  defaultValues: T
): [T, (value: SetQueryStateAction<T>) => void] {
  const [searchParams, setSearchParams] = useSearchParams()

  const resolveValues = useCallback(() => {
    const newState = { ...defaultValues }
    for (const key in defaultValues) {
      const paramValue = searchParams.get(key)
      if (paramValue !== null) {
        newState[key] = paramValue as T[typeof key]
      }
    }
    return newState
  }, [searchParams, defaultValues])

  const [state, setState] = useState<T>(resolveValues)

  useEffect(() => {
    setState(resolveValues())
  }, [searchParams, defaultValues])

  const setQueryState = useCallback(
    (value: SetQueryStateAction<T>) => {
      setState(prevState => {
        const newState = typeof value === 'function' ? value(prevState) : value
        setSearchParams(prevParams => {
          const newParams = new URLSearchParams(prevParams)
          for (const key in newState) {
            if (newState[key] === defaultValues[key]) {
              newParams.delete(key)
            } else {
              newParams.set(key, String(newState[key]))
            }
          }
          return newParams
        })
        return newState
      })
    },
    [defaultValues, setSearchParams]
  )

  return [state, setQueryState]
}
