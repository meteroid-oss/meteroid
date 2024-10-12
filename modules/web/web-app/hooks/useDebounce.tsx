import { useEffect, useState } from 'react'

export function useDebounceValue<T>(value: T, delay: number): T {
  const [debouncedValue, setDebouncedValue] = useState(value)

  useEffect(() => {
    const handler = setTimeout(() => {
      setDebouncedValue(value)
    }, delay)

    return () => {
      clearTimeout(handler)
    }
  }, [value, delay])

  return debouncedValue
}

export function useDebounce<T>(initialValue: T, delay: number): [T, (value: T) => void] {
  const [value, setValue] = useState(initialValue)
  const debouncedValue = useDebounceValue(value, delay)

  return [debouncedValue, setValue]
}
