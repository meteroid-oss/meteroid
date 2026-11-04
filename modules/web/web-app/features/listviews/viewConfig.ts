import { SortingState, VisibilityState } from '@tanstack/react-table'

import { AppFilter } from '../filters/filterTypes'

/**
 * What a saved list view captures (JSON-encoded in `EntityListView.config`; the backend never
 * interprets it). Search is deliberately excluded (ephemeral), as is row density (a global
 * personal preference).
 */
export interface ViewConfig {
  v: 1
  /** Advanced filters (native fields + custom properties). */
  filters: AppFilter[]
  /** The page's quick filters, as their raw URL parameter values (absent = page default). */
  quick: Record<string, string>
  /** Empty = the page's default sort. */
  sort: SortingState
  /** Column visibility; absent = the user's own column preferences. */
  columns?: VisibilityState
}

export const EMPTY_VIEW_CONFIG: ViewConfig = { v: 1, filters: [], quick: {}, sort: [] }

export function serializeViewConfig(config: ViewConfig): string {
  return JSON.stringify(config)
}

const isRecord = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === 'object' && !Array.isArray(v)

/** Tolerant parse: drops malformed parts instead of failing, so one bad view can't break a page. */
export function parseViewConfig(json: string): ViewConfig {
  let raw: unknown
  try {
    raw = JSON.parse(json)
  } catch {
    return EMPTY_VIEW_CONFIG
  }
  if (!isRecord(raw)) return EMPTY_VIEW_CONFIG

  const filters = Array.isArray(raw.filters)
    ? raw.filters.filter(
        (f): f is AppFilter =>
          isRecord(f) &&
          typeof f.field === 'string' &&
          typeof f.op === 'string' &&
          Array.isArray(f.values) &&
          f.values.every(v => typeof v === 'string')
      )
    : []

  const quick: Record<string, string> = {}
  if (isRecord(raw.quick)) {
    for (const [k, v] of Object.entries(raw.quick)) {
      if (typeof v === 'string' && v !== '') quick[k] = v
    }
  }

  const sort = Array.isArray(raw.sort)
    ? raw.sort.filter(
        (s): s is { id: string; desc: boolean } =>
          isRecord(s) && typeof s.id === 'string' && typeof s.desc === 'boolean'
      )
    : []

  let columns: VisibilityState | undefined
  if (isRecord(raw.columns)) {
    columns = {}
    for (const [k, v] of Object.entries(raw.columns)) {
      if (typeof v === 'boolean') columns[k] = v
    }
  }

  return { v: 1, filters, quick, sort, columns }
}

const hiddenIds = (columns: VisibilityState, defaultHidden: string[]): string =>
  Object.entries({ ...Object.fromEntries(defaultHidden.map(id => [id, false])), ...columns })
    .filter(([, visible]) => !visible)
    .map(([id]) => id)
    .sort()
    .join(',')

const filtersKey = (filters: AppFilter[]) =>
  JSON.stringify(filters.map(f => [f.field, f.op, f.values]))

const sortKey = (sort: SortingState) => JSON.stringify(sort.map(s => [s.id, s.desc]))

const sortedEntries = (r: Record<string, string>) =>
  JSON.stringify(
    Object.entries(r)
      .filter(([, v]) => v !== '')
      .sort(([a], [b]) => a.localeCompare(b))
  )

/**
 * Whether the current list state matches a view. Columns are only compared when the view stores
 * them; an empty view sort means the page default.
 */
export function sameViewState(
  current: ViewConfig,
  view: ViewConfig,
  { defaultSorting, defaultHidden }: { defaultSorting: SortingState; defaultHidden: string[] }
): boolean {
  // Structural, not textual: the stored JSONB comes back with its keys reordered.
  if (filtersKey(current.filters) !== filtersKey(view.filters)) return false
  if (sortedEntries(current.quick) !== sortedEntries(view.quick)) return false
  const viewSort = view.sort.length ? view.sort : defaultSorting
  const currentSort = current.sort.length ? current.sort : defaultSorting
  if (sortKey(currentSort) !== sortKey(viewSort)) return false
  if (view.columns && current.columns) {
    if (hiddenIds(current.columns, defaultHidden) !== hiddenIds(view.columns, defaultHidden)) {
      return false
    }
  }
  return true
}
