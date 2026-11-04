import { ReactNode } from 'react'

import { usePermissions } from '@/hooks/usePermissions'

interface GateProps {
  /** Required permission code, e.g. "billing:invoices:manage". `manage` implies `read`. */
  perm: string
  children: ReactNode
  /** Rendered when the permission is absent (default: nothing). */
  fallback?: ReactNode
}

/**
 * Renders `children` only when the current user holds `perm`. When gated out the
 * children are never mounted, so any queries they own never fire (and never 403).
 *
 * UX-only: server-side RBAC remains authoritative. Use for buttons, menu items,
 * modals/sheets and whole sections. For query gating outside JSX, use
 * `usePermissions().has(code)` directly (e.g. in a query's `enabled`).
 */
export const Gate = ({ perm, children, fallback = null }: GateProps) => {
  const { has } = usePermissions()
  return <>{has(perm) ? children : fallback}</>
}
