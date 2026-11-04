import { useCallback, useState } from 'react'

import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { CreditNotesTable, FilterDropdown } from '@/features/creditNotes'
import { useCustomPropertyColumns } from '@/features/customproperties/useCustomPropertyColumns'
import { FilterBar } from '@/features/filters/FilterBar'
import { useEntityFilters } from '@/features/filters/useEntityFilters'
import { useRequestFilters } from '@/features/filters/useRequestFilters'
import { ViewsBar } from '@/features/listviews/ViewsBar'
import { useSavedViews } from '@/features/listviews/useSavedViews'
import { useDebounceValue } from '@/hooks/useDebounce'
import { enumSerde, useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listCreditNotes } from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import { CreditNoteStatus } from '@/rpc/api/creditnotes/v1/models_pb'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { EntityListType } from '@/rpc/api/listviews/v1/models_pb'

import type { SortingState } from '@tanstack/react-table'

const statusSerde = enumSerde(CreditNoteStatus)

const DEFAULT_SORTING: SortingState = [{ id: 'created_at', desc: true }]
// URL params captured by saved views as quick filters.
const QUICK_KEYS = ['status']

export const CreditNotes = () => {
  const [text, setText] = useState('')
  const debouncedText = useDebounceValue(text, 400)
  const [status, setStatus] = useQueryState<CreditNoteStatus | undefined>(
    'status',
    undefined,
    statusSerde
  )

  const [pagination, setPagination] = useListPagination()

  const [sorting, setSorting] = useState<SortingState>(DEFAULT_SORTING)
  const [filters, setFilters] = useEntityFilters()
  const requestFilters = useRequestFilters(CustomPropertyEntityType.CREDIT_NOTE, filters)
  const { hiddenIds: cpHiddenIds } = useCustomPropertyColumns(CustomPropertyEntityType.CREDIT_NOTE)
  const savedViews = useSavedViews({
    listType: EntityListType.CREDIT_NOTE,
    quickKeys: QUICK_KEYS,
    filters,
    sorting,
    setSorting,
    defaultSorting: DEFAULT_SORTING,
    tableKey: 'credit_notes',
    defaultHidden: cpHiddenIds,
  })
  const queryEnabled = savedViews.ready && requestFilters.ready

  useResetPageOnChange(setPagination, [debouncedText, status, filters])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  const creditNotesQuery = useQuery(
    listCreditNotes,
    {
      orderBy: sortingStateToOrderBy(sorting),
      search: debouncedText || undefined,
      status,
      filters: requestFilters.protoFilters,
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
    },
    { enabled: queryEnabled }
  )

  const data = creditNotesQuery.data?.creditNotes ?? []
  const count = creditNotesQuery.data?.paginationMeta?.totalItems ?? 0
  const isLoading = creditNotesQuery.isLoading || !queryEnabled

  return (
    <ListPageLayout
      title="Credit notes"
      count={count}
      footer={<ViewsBar views={savedViews} entityLabel="credit notes" />}
      toolbar={
        <>
          <ListToolbarSearch
            value={text}
            onChange={setText}
            placeholder="Search credit notes"
            className="max-w-xs"
          />
          <FilterDropdown status={status} setStatus={setStatus} />
          <FilterBar
            entityType={CustomPropertyEntityType.CREDIT_NOTE}
            filters={filters}
            onChange={setFilters}
          />
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={isLoading} onClick={() => creditNotesQuery.refetch()} />
      }
    >
      <CreditNotesTable
        data={data}
        totalCount={count}
        pagination={pagination}
        setPagination={setPagination}
        isLoading={isLoading}
        sorting={sorting}
        onSortingChange={handleSortingChange}
        columnState={savedViews.columnState}
      />
    </ListPageLayout>
  )
}
