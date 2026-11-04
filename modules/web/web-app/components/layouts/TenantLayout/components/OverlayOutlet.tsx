import { ReactNode, createContext, useCallback, useContext, useMemo, useRef } from 'react'
import {
  Location,
  To,
  UNSAFE_LocationContext as LocationContext,
  useLocation,
  useMatches,
  useNavigate,
  useOutlet,
  useParams,
} from 'react-router-dom'

import { historyIdx } from '@/lib/utils/history'

type OverlayHandle = { overlay?: boolean }

interface Background {
  outlet: ReactNode
  location: Location
  historyIdx?: number
  tenantSlug?: string
}

const BackgroundContext = createContext<Background | null>(null)

/**
 * An `<Outlet />` where routes flagged `handle.overlay` open on top of the page they were
 * navigated from: that page stays mounted, pinned to its own location, so in-progress work
 * (forms, filters, scroll) survives opening and closing the overlay.
 */
export const OverlayOutlet = () => {
  const outlet = useOutlet()
  const location = useLocation()
  const { tenantSlug } = useParams()
  const { navigationType } = useContext(LocationContext)
  const isOverlay = useMatches().some(m => (m.handle as OverlayHandle | undefined)?.overlay)
  const background = useRef<Background | null>(null)

  if (!isOverlay) {
    background.current = { outlet, location, historyIdx: historyIdx(), tenantSlug }
  } else if (background.current?.tenantSlug !== tenantSlug) {
    background.current = null
  }

  const page = isOverlay ? background.current : { outlet, location }
  const pageLocation = page?.location
  const pageLocationContext = useMemo(
    () => pageLocation && { location: pageLocation, navigationType },
    [pageLocation, navigationType]
  )

  return (
    <BackgroundContext.Provider value={isOverlay ? background.current : null}>
      {page && pageLocationContext && (
        <LocationContext.Provider value={pageLocationContext}>{page.outlet}</LocationContext.Provider>
      )}
      {isOverlay && outlet}
    </BackgroundContext.Provider>
  )
}

/**
 * Close the current overlay: step back through history to the page it was opened over, or,
 * opened by direct link, replace it with `fallback` so Back doesn't reopen it.
 */
export const useCloseOverlay = (fallback: To) => {
  const background = useContext(BackgroundContext)
  const navigate = useNavigate()

  return useCallback(() => {
    const current = historyIdx()
    const target = background?.historyIdx
    if (current !== undefined && target !== undefined && target < current) {
      navigate(target - current)
    } else {
      navigate(background?.location ?? fallback, { replace: true })
    }
  }, [background, fallback, navigate])
}
