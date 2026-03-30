import type { SortingState } from '@tanstack/react-table'

export function sortingStateToOrderBy(
  sorting: SortingState,
  defaultOrderBy?: string
): string | undefined {
  if (sorting.length === 0) return defaultOrderBy
  const { id, desc } = sorting[0]
  return `${id}.${desc ? 'desc' : 'asc'}`
}
