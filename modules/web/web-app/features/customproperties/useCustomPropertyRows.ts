import { useQuery } from '@/lib/connectrpc'
import { listCustomPropertyDefinitions } from '@/rpc/api/customproperties/v1/customproperties-CustomPropertiesService_connectquery'
import {
  CustomPropertyDefinition,
  CustomPropertyEntityType,
  CustomPropertyType,
} from '@/rpc/api/customproperties/v1/models_pb'

import { parseCustomProperties } from './CustomPropertiesFields'

export interface CustomPropertyRow {
  key: string
  label: string
  /** Pre-formatted text (select labels resolved). Use for plain-text contexts (tables, exports). */
  value: string
  /** The definition and raw stored value, for typed rendering (e.g. mailto/URL links). */
  def: CustomPropertyDefinition
  raw: unknown
}

const hasValue = (v: unknown): boolean =>
  v !== undefined && v !== null && v !== '' && !(Array.isArray(v) && v.length === 0)

/** Formats a stored custom-property value for read-only display, resolving select labels. */
export function formatCustomPropertyValue(def: CustomPropertyDefinition, value: unknown): string {
  const options = def.config?.options ?? []
  const labelFor = (v: string) => options.find(o => o.value === v)?.label || v
  switch (def.propertyType) {
    case CustomPropertyType.BOOLEAN:
      return value ? 'Yes' : 'No'
    case CustomPropertyType.SINGLE_SELECT:
      return labelFor(String(value))
    case CustomPropertyType.MULTI_SELECT:
      return Array.isArray(value) ? value.map(v => labelFor(String(v))).join(', ') : String(value)
    default:
      // TEXT, NUMBER, DATE, DATETIME, JSON, URL, EMAIL
      return typeof value === 'object' ? JSON.stringify(value) : String(value)
  }
}

/**
 * Read-only custom-property rows for an entity: the tenant's active definitions that have a
 * value on this entity, ordered by definition `displayOrder`. Shares the definitions query
 * (same params) with the edit form, so no extra round-trip.
 */
export function useCustomPropertyRows(
  entityType: CustomPropertyEntityType,
  customProperties?: string
): CustomPropertyRow[] {
  const { data } = useQuery(listCustomPropertyDefinitions, {
    entityType,
    includeArchived: false,
  })
  const values = parseCustomProperties(customProperties)
  return (data?.definitions ?? [])
    .filter(def => hasValue(values[def.key]))
    .sort((a, b) => Number(a.displayOrder) - Number(b.displayOrder))
    .map(def => ({
      key: def.key,
      label: def.name,
      value: formatCustomPropertyValue(def, values[def.key]),
      def,
      raw: values[def.key],
    }))
}
