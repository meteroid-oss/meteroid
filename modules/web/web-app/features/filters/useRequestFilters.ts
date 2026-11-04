import { useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

import { AppFilter, MEMBER_ME, VALUELESS_OPS } from './filterTypes'
import { toProtoFilters } from './useEntityFilters'
import { useFilterFieldsQuery } from './useFilterFields'

/** Whether a filter is complete enough to send (a value-taking operator needs its values). */
function isComplete(filter: AppFilter): boolean {
  if (VALUELESS_OPS.has(filter.op)) return true
  if (filter.op === 'between') return filter.values.length === 2 && filter.values.every(Boolean)
  return filter.values.some(v => v !== '')
}

/**
 * The filters to send with a list request. Filters on a field that no longer exists (e.g. an
 * archived custom property carried by a saved view or an old link) or with an operator the field
 * no longer allows are dropped instead of failing the whole request; the filter bar flags them.
 *
 * Member values of {@link MEMBER_ME} become the caller's user id.
 *
 * `ready` is false while the field definitions (or the caller, for "me") load and filters are
 * present, so the list query can wait rather than briefly render unfiltered (or error) results.
 */
export function useRequestFilters(entityType: CustomPropertyEntityType, filters: AppFilter[]) {
  const { fields, isLoaded } = useFilterFieldsQuery(entityType)
  const meId = useQuery(me, undefined, { staleTime: 5 * 60_000 }).data?.user?.id

  return useMemo(() => {
    const byField = new Map(fields.map(f => [f.field, f]))
    const applicable = filters.filter(f => {
      const field = byField.get(f.field)
      return !!field && field.ops.includes(f.op) && isComplete(f)
    })
    const needsMe = applicable.some(
      f => byField.get(f.field)?.widget === 'member' && f.values.includes(MEMBER_ME)
    )
    const resolved = applicable.map(f =>
      byField.get(f.field)?.widget === 'member'
        ? { ...f, values: f.values.map(v => (v === MEMBER_ME ? (meId ?? v) : v)) }
        : f
    )
    return {
      protoFilters: toProtoFilters(resolved),
      ready: (isLoaded || filters.length === 0) && (!needsMe || !!meId),
    }
  }, [fields, isLoaded, filters, meId])
}
