import { useCallback, useEffect, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

type SetQueryStateAction<T> = T | ((prevState: T) => T)

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

  useEffect(() => {
    const paramValue = searchParams.get(key)
    if (paramValue !== null) {
      try {
        setState(deserialize(paramValue))
      } catch {
        // If deserialization fails, we keep the current state
      }
    } else {
      setState(defaultValue)
    }
  }, [searchParams, key, deserialize, defaultValue])

  const setQueryState = useCallback(
    (value: SetQueryStateAction<T>) => {
      setState(prevState => {
        const newState =
          typeof value === 'function' ? (value as (prevState: T) => T)(prevState) : value

        setSearchParams(prevParams => {
          const newParams = new URLSearchParams(prevParams)
          if (newState === defaultValue) {
            newParams.delete(key)
          } else {
            newParams.set(key, serialize(newState))
          }
          return newParams
        })

        return newState
      })
    },
    [key, setSearchParams, serialize, defaultValue]
  )

  return [state, setQueryState]
}
