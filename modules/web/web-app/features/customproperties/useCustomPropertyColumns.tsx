import { ColumnDef } from '@tanstack/react-table'

import { useQuery } from '@/lib/connectrpc'
import { listCustomPropertyDefinitions } from '@/rpc/api/customproperties/v1/customproperties-CustomPropertiesService_connectquery'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'

import { parseCustomProperties } from './CustomPropertiesFields'
import { formatCustomPropertyValue } from './useCustomPropertyRows'

const hasValue = (v: unknown): boolean =>
  v !== undefined && v !== null && v !== '' && !(Array.isArray(v) && v.length === 0)

/** Stable column id for a custom-property definition key. */
export const customPropertyColumnId = (key: string) => `cp_${key}`

/** Id of the trailing row-actions ("⋮") column, so custom-property columns can be
 *  inserted before it rather than after. */
export const ACTIONS_COLUMN_ID = 'actions'

/**
 * Append custom-property columns to a table's base columns, placing them *before* the
 * trailing row-actions column (id {@link ACTIONS_COLUMN_ID}) so the "⋮" menu stays
 * rightmost. Falls back to a plain append when the table has no actions column.
 */
export function withCustomPropertyColumns<T>(
  base: ColumnDef<T>[],
  customProperties: ColumnDef<T>[]
): ColumnDef<T>[] {
  if (customProperties.length === 0) return base
  const actionsIdx = base.findIndex(c => c.id === ACTIONS_COLUMN_ID)
  if (actionsIdx === -1) return [...base, ...customProperties]
  return [...base.slice(0, actionsIdx), ...customProperties, ...base.slice(actionsIdx)]
}

/**
 * Optional table columns — one per active custom-property definition for `entityType`,
 * each rendering that property's value for the row. Feed the returned `hiddenIds` to the
 * table's `defaultHidden` so the columns are opt-in (toggled from the table's view menu).
 *
 * The row type must expose `customProperties` (the JSON-encoded string carried by the
 * customer/subscription/invoice list protos).
 */
export function useCustomPropertyColumns<T extends { customProperties?: string }>(
  entityType: CustomPropertyEntityType
): { columns: ColumnDef<T>[]; hiddenIds: string[] } {
  const { data } = useQuery(listCustomPropertyDefinitions, {
    entityType,
    includeArchived: false,
  })
  const definitions = data?.definitions ?? []

  // The view-options menu labels columns by their string `header` (falling back from
  // `meta.label`, which isn't typed on ColumnMeta here), so a plain string header is enough.
  const columns: ColumnDef<T>[] = definitions.map(def => ({
    id: customPropertyColumnId(def.key),
    header: def.name,
    enableSorting: false,
    accessorFn: (row: T) => {
      const value = parseCustomProperties(row.customProperties)[def.key]
      return hasValue(value) ? formatCustomPropertyValue(def, value) : ''
    },
  }))

  return { columns, hiddenIds: columns.map(c => c.id as string) }
}
