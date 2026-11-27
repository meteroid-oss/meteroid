import { useCallback, useSyncExternalStore } from 'react'

import { useTenant } from '@/hooks/useTenant'

/** What the Launch pad remembers per tenant, in this browser. */
interface GuidePrefs {
  /** The user put the guide away; the organisation menu brings it back. */
  hidden: boolean
  /** Steps without a data signal count as done once their screen was opened from the guide. */
  visited: string[]
  /** The guide opened by itself once, on the first visit to Home. */
  autoOpened: boolean
}

const DEFAULTS: GuidePrefs = { hidden: false, visited: [], autoOpened: false }
// Fired on writes, so every component reading the prefs updates (the `storage` event only reaches
// other tabs).
const CHANGE_EVENT = 'meteroid:guide-prefs'

const storageKey = (tenantId: string) => `meteroid:guide:${tenantId}`

const read = (key: string): GuidePrefs => {
  try {
    const raw = window.localStorage.getItem(key)
    return raw ? { ...DEFAULTS, ...JSON.parse(raw) } : DEFAULTS
  } catch {
    return DEFAULTS
  }
}

const subscribe = (onChange: () => void) => {
  window.addEventListener('storage', onChange)
  window.addEventListener(CHANGE_EVENT, onChange)
  return () => {
    window.removeEventListener('storage', onChange)
    window.removeEventListener(CHANGE_EVENT, onChange)
  }
}

export const useGuidePrefs = () => {
  const { tenant } = useTenant()
  const key = tenant ? storageKey(tenant.id) : undefined

  // The raw string is the snapshot: stable between writes, so React doesn't re-render needlessly.
  const raw = useSyncExternalStore(subscribe, () => (key ? window.localStorage.getItem(key) : null))
  const prefs: GuidePrefs = key && raw !== null ? read(key) : DEFAULTS

  const update = useCallback(
    (patch: (current: GuidePrefs) => Partial<GuidePrefs>) => {
      if (!key) return
      const current = read(key)
      window.localStorage.setItem(key, JSON.stringify({ ...current, ...patch(current) }))
      window.dispatchEvent(new Event(CHANGE_EVENT))
    },
    [key]
  )

  return {
    ...prefs,
    ready: !!key,
    setHidden: useCallback((hidden: boolean) => update(() => ({ hidden })), [update]),
    markVisited: useCallback(
      (stepId: string) =>
        update(current =>
          current.visited.includes(stepId) ? {} : { visited: [...current.visited, stepId] }
        ),
      [update]
    ),
    markAutoOpened: useCallback(() => update(() => ({ autoOpened: true })), [update]),
  }
}
