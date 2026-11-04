import { useQueryState } from '@/hooks/useQueryState'
import { FilterOperator } from '@/rpc/common/v1/filters_pb'

import { AppFilter, FilterOp } from './filterTypes'

const OP_TO_PROTO: Record<FilterOp, FilterOperator> = {
  equals: FilterOperator.EQUALS,
  not_equals: FilterOperator.NOT_EQUALS,
  in: FilterOperator.IN,
  contains: FilterOperator.CONTAINS,
  gt: FilterOperator.GT,
  gte: FilterOperator.GTE,
  lt: FilterOperator.LT,
  lte: FilterOperator.LTE,
  between: FilterOperator.BETWEEN,
  is_set: FilterOperator.IS_SET,
  is_empty: FilterOperator.IS_EMPTY,
  is_true: FilterOperator.IS_TRUE,
  is_false: FilterOperator.IS_FALSE,
  before: FilterOperator.BEFORE,
  after: FilterOperator.AFTER,
}

/** Map UI filters to the wire `Filter` shape for a connect-query request. */
export function toProtoFilters(filters: AppFilter[]) {
  return filters.map(f => ({ field: f.field, op: OP_TO_PROTO[f.op], values: f.values }))
}

const EMPTY_FILTERS: AppFilter[] = []

/** URL (de)serialization of a filter list; shared with saved views so they write the same format. */
export const FILTERS_SERDE = {
  serialize: (filters: AppFilter[]): string =>
    filters.map(f => `${f.field}~${f.op}~${f.values.map(encodeURIComponent).join('|')}`).join(';'),
  deserialize: (raw: string): AppFilter[] =>
    raw
      ? raw.split(';').map(part => {
          const [field = '', op = '', vals] = part.split('~')
          return {
            field,
            op: op as FilterOp,
            values: vals ? vals.split('|').map(decodeURIComponent) : [],
          }
        })
      : [],
}

/** URL-synced list-filter state for an entity list page. */
/** URL parameter holding the advanced filters. */
export const FILTERS_PARAM = 'filters'

export function useEntityFilters(key = FILTERS_PARAM) {
  return useQueryState<AppFilter[]>(key, EMPTY_FILTERS, FILTERS_SERDE)
}
