import { createConnectQueryKey } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useMemo } from 'react'

import { useLiveSubscriptions } from '@/features/credit/useLiveSubscriptions'
import { useQuery } from '@/lib/connectrpc'
import { listResourceTypes } from '@/rpc/api/resources/v1/resources-ResourceTypesService_connectquery'
import { listResources } from '@/rpc/api/resources/v1/resources-ResourcesService_connectquery'

import type { Resource, ResourceType } from '@/rpc/api/resources/v1/models_pb'

/** The tenant's resource types, archived ones included. Without any, no resource UI shows. */
export function useResourceTypes() {
  const query = useQuery(listResourceTypes, { includeArchived: true })
  return useMemo(() => {
    const all = [...(query.data?.resourceTypes ?? [])].sort((a, b) => a.name.localeCompare(b.name))
    return {
      all,
      types: all.filter(t => !t.archived),
      byId: new Map(all.map(t => [t.id, t])),
      hasTypes: all.length > 0,
      isLoading: query.isLoading,
    }
  }, [query.data, query.isLoading])
}

/** A customer's stored resources, of one type or all. */
export function useCustomerResources(customerId: string | undefined, typeId?: string) {
  const query = useQuery(
    listResources,
    { customerId, resourceType: typeId, pagination: { page: 0, perPage: 500 } },
    { enabled: !!customerId }
  )
  return useMemo(() => {
    const resources = query.data?.resources ?? []
    return {
      resources,
      byId: new Map(resources.map(r => [r.id, r])),
      isLoading: query.isLoading,
    }
  }, [query.data, query.isLoading])
}

/** "Site s2" by resource id, for lists naming what each subscription is for (one customer, or all). */
export function useResourceChips(customerId?: string) {
  const { byId: types, hasTypes } = useResourceTypes()
  const query = useQuery(
    listResources,
    { customerId, includeArchived: true, pagination: { page: 0, perPage: 500 } },
    { enabled: hasTypes }
  )
  return useMemo(
    () =>
      new Map(
        (query.data?.resources ?? []).map(r => [r.id, resourceChip(r, types.get(r.resourceTypeId))!])
      ),
    [query.data, types]
  )
}

/** A customer's live subscriptions, and which are for one resource or the whole customer. */
export function useResourceSubscriptions(customerId: string | undefined) {
  const live = useLiveSubscriptions(customerId ?? '', !!customerId)
  return {
    live,
    byId: new Map(live.map(s => [s.id, s])),
    forResource: (resourceId?: string) => live.filter(s => s.resourceId === resourceId),
  }
}

export const useInvalidateResources = () => {
  const queryClient = useQueryClient()
  return () =>
    Promise.all(
      [listResources.parent, listResourceTypes.parent].map(schema =>
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema, cardinality: undefined }),
        })
      )
    )
}

export const lower = (s: string) => s.toLowerCase()

/** "Marketing (s4)", or the key value alone. */
export const resourceLabel = (r: Pick<Resource, 'name' | 'keyValue'>) =>
  r.name ? `${r.name} (${r.keyValue})` : r.keyValue

/** "Site s2": how lists and chips name what a subscription is for. */
export const resourceChip = (r: Resource | undefined, type: ResourceType | undefined) =>
  r ? `${type?.name ?? 'Resource'} ${r.name || r.keyValue}` : undefined

/** The check's `resources` object for a resource: its type's canonical key. */
export const resourceSelector = (type: ResourceType, keyValue: string) => ({
  [type.keys[0]]: keyValue,
})

/** "Site ID" for `site_id`, "Environment" for `environment`: a code as words. */
export const humanize = (code: string) => {
  const words = code
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .split(/[\s_.-]+/)
    .filter(Boolean)
    .map(w => (/^(id|url|ip)$/i.test(w) ? w.toUpperCase() : w.toLowerCase()))
  const text = words.join(' ')
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/** "Site ID": the label of a type's canonical key. */
export const keyLabel = (type: ResourceType) => humanize(type.keys[0] ?? `${type.code}_id`)

/** Types by id, or a list of them. */
export type TypeList = ReadonlyMap<string, ResourceType> | readonly ResourceType[]

const typeByCode = (types: TypeList, code: string) => [...types.values()].find(t => t.code === code)

/** "Site environment" for the attribute key `site.environment`. */
export const attributeKeyLabel = (key: string, types: TypeList) => {
  const [code, ...rest] = key.split('.')
  const type = typeByCode(types, code)
  if (!type || !rest.length) return humanize(key)
  const attribute = humanize(rest.join('.'))
  return `${type.name} ${/^[A-Z]{2}/.test(attribute) ? attribute : attribute.toLowerCase()}`
}

/** "Site Marketing" for the path `site=s1`, by the customer's stored resource when known. */
export const pathLabel = (path: string, types: TypeList, resources: Resource[] = []) => {
  const [code, ...rest] = path.split('=')
  const key = rest.join('=')
  const type = typeByCode(types, code)
  if (!type) return path
  const stored = resources.find(r => r.resourceTypeId === type.id && r.keyValue === key)
  return resourceChip(stored, type) ?? `${type.name} ${key}`
}

export function usePathLabel(customerId: string | undefined) {
  const { all } = useResourceTypes()
  const { resources } = useCustomerResources(customerId)
  return useCallback((path: string) => pathLabel(path, all, resources), [all, resources])
}

export const attributesLabel = (attributes: Record<string, string>) =>
  Object.entries(attributes)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([, v]) => v)
    .join(' · ')
