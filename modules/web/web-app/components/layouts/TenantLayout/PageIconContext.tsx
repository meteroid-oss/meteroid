import { ComponentType, createContext, useContext } from 'react'

import { HomeIcon } from '@/components/icons/AppIcons'

type PageIcon = ComponentType<{ className?: string }>

/** The nav icon of the page you are on (its own, or its section's), set by TenantLayout. */
export const PageIconContext = createContext<PageIcon>(HomeIcon)

export const usePageIcon = () => useContext(PageIconContext)

interface NavEntry {
  url?: unknown
  icon?: PageIcon
  items?: { url?: unknown }[]
}

/**
 * The icon of the nav entry the current path belongs to. Matches like the nav does (the path
 * contains the entry's url); of several matches the longest url wins. Sub-items have no icon of
 * their own and take their section's. Home, and anything without a match, gets the home icon.
 */
export const resolvePageIcon = (pathname: string, entries: NavEntry[]): PageIcon => {
  const candidates = entries.flatMap(({ url, icon, items }) =>
    icon
      ? [url, ...(items ?? []).map(sub => sub.url)]
          .filter((u): u is string => typeof u === 'string' && u !== '')
          .map(u => ({ url: u, icon }))
      : []
  )
  const match = candidates
    .filter(c => c.url !== '.' && pathname.includes(c.url))
    .sort((a, b) => b.url.length - a.url.length)[0]
  return match?.icon ?? HomeIcon
}
