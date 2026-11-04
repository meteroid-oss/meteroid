import { useMemo } from 'react'

import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'

import { AddFilterPopover } from './AddFilterPopover'
import { FilterChip } from './FilterChip'
import { AppFilter } from './filterTypes'
import { ResolvedField, useFilterFieldsQuery } from './useFilterFields'

interface Props {
  entityType: CustomPropertyEntityType
  filters: AppFilter[]
  onChange: (filters: AppFilter[]) => void
  /**
   * A page's own fields, listed first under their own group. The page maps their filters to
   * its request itself: they are not entity filters (keep them out of `useRequestFilters`).
   */
  localFields?: ResolvedField[]
  /** One heading for every entity field (native and custom) when the page adds its own. */
  entityGroup?: string
}

/**
 * Toolbar filter control: a chip per active filter plus a single "＋ Filter" entry point. Fields
 * are native columns plus the tenant's custom-property definitions for `entityType`, after the
 * page's own `localFields`.
 */
export const FilterBar = ({ entityType, filters, onChange, localFields, entityGroup }: Props) => {
  const query = useFilterFieldsQuery(entityType)
  const isLoaded = query.isLoaded
  const fields = useMemo(
    () => [
      ...(localFields ?? []),
      ...(entityGroup ? query.fields.map(f => ({ ...f, group: entityGroup })) : query.fields),
    ],
    [localFields, entityGroup, query.fields]
  )
  const fieldByKey = useMemo(() => new Map(fields.map(f => [f.field, f])), [fields])

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {filters.map((filter, i) => (
        <FilterChip
          key={`${filter.field}-${i}`}
          filter={filter}
          field={fieldByKey.get(filter.field)}
          loaded={isLoaded}
          onChange={next => onChange(filters.map((f, j) => (j === i ? next : f)))}
          onRemove={() => onChange(filters.filter((_, j) => j !== i))}
        />
      ))}
      <AddFilterPopover fields={fields} onAdd={filter => onChange([...filters, filter])} />
    </div>
  )
}
