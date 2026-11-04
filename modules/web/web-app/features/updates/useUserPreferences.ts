import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useRef } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { me, updateUserPreferences } from '@/rpc/api/users/v1/users-UsersService_connectquery'

/**
 * Per-user preferences bag. An opaque JSON object owned by the frontend and
 * persisted server-side (shallow-merged per top-level key). Add new UI prefs
 * here — e.g. table column ordering — without a backend change.
 */
export interface UserPreferences {
  /** ISO date of the newest changelog entry the user has seen. */
  changelogSeenAt?: string
  /** Slug of the organization the user last actively viewed. */
  lastOrganizationSlug?: string
  /** Last tenant the user actively viewed, keyed by organization slug. */
  lastTenantByOrg?: Record<string, string>
}

export const useUserPreferences = () => {
  const queryClient = useQueryClient()
  const meQuery = useQuery(me)

  const preferences = useMemo<UserPreferences>(() => {
    const raw = meQuery.data?.user?.preferences
    if (!raw) return {}
    try {
      return JSON.parse(raw) as UserPreferences
    } catch {
      return {}
    }
  }, [meQuery.data?.user?.preferences])

  const mutation = useMutation(updateUserPreferences, {
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: me, cardinality: 'finite' }),
      })
    },
  })

  const update = useCallback(
    (patch: Partial<UserPreferences>) => mutation.mutateAsync({ preferences: JSON.stringify(patch) }),
    [mutation]
  )

  return { preferences, update, isLoading: meQuery.isLoading }
}

/**
 * Persists the org/tenant the user is actively viewing so the root redirects
 * can lead back here next time instead of falling back to the first workspace.
 * The tenant is keyed by org slug so multi-org users resume the right tenant in
 * each. Writes only when the stored value changes, and at most once per pair.
 */
export const useTrackLastVisited = (organizationSlug?: string, tenantSlug?: string) => {
  const { preferences, update, isLoading } = useUserPreferences()
  const lastWritten = useRef<string | null>(null)

  useEffect(() => {
    if (isLoading || !organizationSlug) return

    const key = `${organizationSlug}::${tenantSlug ?? ''}`
    if (lastWritten.current === key) return
    lastWritten.current = key

    const patch: Partial<UserPreferences> = {}
    if (preferences.lastOrganizationSlug !== organizationSlug) {
      patch.lastOrganizationSlug = organizationSlug
    }
    if (tenantSlug && preferences.lastTenantByOrg?.[organizationSlug] !== tenantSlug) {
      patch.lastTenantByOrg = { ...preferences.lastTenantByOrg, [organizationSlug]: tenantSlug }
    }

    if (Object.keys(patch).length > 0) void update(patch)
  }, [organizationSlug, tenantSlug, preferences, update, isLoading])
}
