import { useEffect } from 'react'
import { useMatches } from 'react-router-dom'

type TitleHandle = { title?: string | ((match: { params: Record<string, string | undefined> }) => string) }

const BASE = 'Meteroid'

export const usePageTitle = () => {
  const matches = useMatches()

  useEffect(() => {
    let resolved: string | undefined
    for (let i = matches.length - 1; i >= 0; i--) {
      const handle = matches[i].handle as TitleHandle | undefined
      if (!handle?.title) continue
      resolved =
        typeof handle.title === 'function'
          ? handle.title({ params: matches[i].params })
          : handle.title
      if (resolved) break
    }
    document.title = resolved ? `${resolved} · ${BASE}` : BASE
  }, [matches])
}
