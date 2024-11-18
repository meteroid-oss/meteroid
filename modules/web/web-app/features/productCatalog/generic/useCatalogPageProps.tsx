import { PaginationState } from '@tanstack/react-table'

import { useDebounceValue } from '@/hooks/useDebounce'
import { SetQueryStateAction, useQueryRecordState, useQueryState } from '@/hooks/useQueryState'
import { useTypedParams } from '@/utils/params'

export const useCatalogPageProps = () => {
  const paginationState = useQueryRecordState({
    pageIndex: 0,
    pageSize: 20,
  })

  const [pagination] = paginationState

  const [_search, onSearch] = useQueryState<string | undefined>('q', undefined)

  const search = useDebounceValue<string | undefined>(_search, 300)

  const { familyLocalId } = useTypedParams<{ familyLocalId: string }>()

  const paginationQuery = {
    limit: pagination.pageSize,
    offset: pagination.pageIndex * pagination.pageSize,
  }

  return {
    baseQuery: familyLocalId
      ? {
          productFamilyLocalId: familyLocalId,
          pagination: paginationQuery,
          search: search,
        }
      : undefined,

    paginationState,
    onSearch,
  }
}

export type UsePaginationState = [
  PaginationState,
  (value: SetQueryStateAction<PaginationState>) => void,
]
