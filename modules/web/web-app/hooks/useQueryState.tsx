import { useCallback, useEffect, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

export type SetQueryStateAction<T> = T | ((prevState: T) => T)

export const ARRAY_SERDE = {
  serialize: (value: string[]) => value.join(','),
  deserialize: (value: string) => value.split(','),
}

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
  }: {
    serialize?: (value: T) => string
    deserialize?: (value: string) => T
  } = {}
): [T, (value: SetQueryStateAction<T>) => void] {
  const [searchParams, setSearchParams] = useSearchParams()
  const [state, setState] = useState<T>(() => {
    const paramValue = searchParams.get(key)
    if (paramValue === null) {
      return defaultValue
    }
    try {
      return deserialize(paramValue)
    } catch {
      return defaultValue
    }
  })

  const setQueryState = useCallback(
    (value: SetQueryStateAction<T>) => {
      const newState = typeof value === 'function' ? (value as (prevState: T) => T)(state) : value

      setState(newState)
      setSearchParams(prevParams => {
        const newParams = new URLSearchParams(prevParams)
        let serialized = null
        if (
          newState === defaultValue ||
          newState === '' ||
          (serialized = serialize(newState)) == ''
        ) {
          newParams.delete(key)
        } else {
          newParams.set(key, serialized)
        }
        return newParams
      })
    },
    [key, setSearchParams, serialize, defaultValue, state]
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
