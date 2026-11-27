import { useMemo } from 'react'

import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'

import { AddFilterPopover } from './AddFilterPopover'
import { FilterChip } from './FilterChip'
import { AppFilter, FilterOp } from './filterTypes'
import { ResolvedField, useFilterFieldsQuery } from './useFilterFields'

/**
 * A page's own filter (a status, a type), bound to the state the page already keeps for it: the
 * chip reads `values` and writes back through `set`, so the page's request and URL stay as they
 * were. Empty `values` means the filter is off.
 */
export interface LocalFilter {
  field: ResolvedField
  values: string[]
  set: (values: string[]) => void
  /** The operator the chip shows; the field's first by default. */
  op?: FilterOp
}

/** A choice from a fixed list: one value (`equals`) or several (`in`). */
export const optionField = (
  field: string,
  label: string,
  options: { value: string; label: string; group?: string }[],
  { multi = true, group = 'Filters' }: { multi?: boolean; group?: string } = {}
): ResolvedField => ({
  field,
  label,
  widget: 'select',
  ops: [multi ? 'in' : 'equals'],
  options,
  group,
})

interface Props {
  /** The entity whose native columns and custom properties can be filtered on. */
  entityType?: CustomPropertyEntityType
  /** The entity filters, as the request sends them (see `useRequestFilters`). */
  filters?: AppFilter[]
  onChange?: (filters: AppFilter[]) => void
  /** The page's own filters, listed first in the picker and as the first chips. */
  local?: LocalFilter[]
  /** One heading for every entity field (native and custom) when the page adds its own. */
  entityGroup?: string
}

/**
 * The toolbar's filters: one "Filter" entry point that stays in place, and a chip per active
 * filter after it. Fields are the page's own (`local`), then the entity's native columns and
 * the tenant's custom properties for `entityType`.
 */
export const FilterBar = ({
  entityType,
  filters = [],
  onChange,
  local = [],
  entityGroup,
}: Props) => {
  const query = useFilterFieldsQuery(entityType)
  const isLoaded = query.isLoaded
  const localKeys = new Set(local.map(l => l.field.field))
  const fields = useMemo(
    () => [
      ...local.map(l => l.field),
      ...(entityGroup ? query.fields.map(f => ({ ...f, group: entityGroup })) : query.fields),
    ],
    // `local` is rebuilt each render; its fields are what matter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [local.map(l => l.field.field).join(), entityGroup, query.fields]
  )
  const fieldByKey = new Map(fields.map(f => [f.field, f]))

  const all: AppFilter[] = [
    ...local
      .filter(l => l.values.length > 0)
      .map(l => ({ field: l.field.field, op: l.op ?? l.field.ops[0], values: l.values })),
    ...filters,
  ]

  // Each page filter takes its values back from the list; the rest are the entity's.
  const update = (next: AppFilter[]) => {
    for (const l of local) {
      const values = next.find(f => f.field === l.field.field)?.values ?? []
      const current = l.values.join('\u0000')
      if (values.join('\u0000') !== current) l.set(values)
    }
    const entity = next.filter(f => !localKeys.has(f.field))
    if (onChange && JSON.stringify(entity) !== JSON.stringify(filters)) onChange(entity)
  }

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
      <AddFilterPopover
        fields={fields}
        // A page filter is one chip: picking it again edits that chip rather than adding one.
        onAdd={filter =>
          update(
            localKeys.has(filter.field)
              ? [...all.filter(f => f.field !== filter.field), filter]
              : [...all, filter]
          )
        }
      />
      {all.map((filter, i) => (
        <FilterChip
          key={`${filter.field}-${i}`}
          filter={filter}
          field={fieldByKey.get(filter.field)}
          loaded={isLoaded}
          onChange={next => update(all.map((f, j) => (j === i ? next : f)))}
          onRemove={() => update(all.filter((_, j) => j !== i))}
        />
      ))}
    </div>
  )
}
