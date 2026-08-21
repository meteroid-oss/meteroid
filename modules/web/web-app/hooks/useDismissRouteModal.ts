import { useCallback } from 'react'
import { useLocation, useNavigate } from 'react-router'

/**
 * Dismiss a push-opened route-modal by popping its history entry, not pushing a
 * fresh parent — a push leaves the modal in the back-stack for a parent's
 * `navigate(-1)` to reopen. `default` key = deep link (nothing to pop) → '..'.
 */
export const useDismissRouteModal = () => {
  const navigate = useNavigate()
  const location = useLocation()

  return useCallback(
    () => (location.key === 'default' ? navigate('..') : navigate(-1)),
    [location.key, navigate]
  )
}
