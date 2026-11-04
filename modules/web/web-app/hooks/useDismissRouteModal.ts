import { useCallback } from 'react'
import { useNavigate } from 'react-router'

import { historyIdx } from '@/lib/utils/history'

/**
 * Dismiss a push-opened route-modal by popping its history entry, not pushing a
 * fresh parent — a push leaves the modal in the back-stack for a parent's
 * `navigate(-1)` to reopen. Deep link (nothing to pop) → replace with '..'.
 */
export const useDismissRouteModal = () => {
  const navigate = useNavigate()

  return useCallback(
    () => (historyIdx() ? navigate(-1) : navigate('..', { replace: true })),
    [navigate]
  )
}
