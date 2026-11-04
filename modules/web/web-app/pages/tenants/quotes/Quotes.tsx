import { Button } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { Plus } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Link } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { BaseFilter } from '@/features/TablePage'
import { useCustomPropertyColumns } from '@/features/customproperties/useCustomPropertyColumns'
import { FilterBar } from '@/features/filters/FilterBar'
import { useEntityFilters } from '@/features/filters/useEntityFilters'
import { useRequestFilters } from '@/features/filters/useRequestFilters'
import { ViewsBar } from '@/features/listviews/ViewsBar'
import { useSavedViews } from '@/features/listviews/useSavedViews'
import { QuotesTable } from '@/features/quotes/QuotesTable'
import { useBasePath } from '@/hooks/useBasePath'
import { useDebounceValue } from '@/hooks/useDebounce'
import { enumSerde, useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { EntityListType } from '@/rpc/api/listviews/v1/models_pb'
import { QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import { listQuotes } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

const statusSerde = enumSerde(QuoteStatus)

const DEFAULT_SORTING: SortingState = [{ id: 'created_at', desc: true }]
// URL params captured by saved views as quick filters.
const QUICK_KEYS = ['status']

export const Quotes = () => {
  const basePath = useBasePath()

  const [search, setSearch] = useState('')
  const debouncedSearch = useDebounceValue(search, 400)
  const [status, setStatus] = useQueryState<QuoteStatus | undefined>(
    'status',
    undefined,
    statusSerde
  )
  const [sorting, setSorting] = useState<SortingState>(DEFAULT_SORTING)
  const [pagination, setPagination] = useListPagination()
  const [filters, setFilters] = useEntityFilters()
  const requestFilters = useRequestFilters(CustomPropertyEntityType.QUOTE, filters)
  const { hiddenIds: cpHiddenIds } = useCustomPropertyColumns(CustomPropertyEntityType.QUOTE)
  const savedViews = useSavedViews({
    listType: EntityListType.QUOTE,
    quickKeys: QUICK_KEYS,
    filters,
    sorting,
    setSorting,
    defaultSorting: DEFAULT_SORTING,
    tableKey: 'quotes',
    defaultHidden: cpHiddenIds,
  })
  const queryEnabled = savedViews.ready && requestFilters.ready

  useResetPageOnChange(setPagination, [debouncedSearch, status, filters])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  const quotesQuery = useQuery(
    listQuotes,
    {
      search: debouncedSearch || undefined,
      status,
      filters: requestFilters.protoFilters,
      orderBy: sortingStateToOrderBy(sorting),
      pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
    },
    { enabled: queryEnabled }
  )

  const quotes = quotesQuery.data?.quotes || []
  const totalCount = Number(quotesQuery.data?.paginationMeta?.totalItems ?? 0)
  const isLoading = quotesQuery.isLoading || !queryEnabled

  return (
    <ListPageLayout
      title="Quotes"
      count={totalCount}
      actions={
        <Gate perm="billing:quotes:manage">
          <Button asChild variant="primary" size="sm" hasIcon>
            <Link to={`${basePath}/quotes/create`}>
              <Plus className="w-4 h-4" />
              New Quote
            </Link>
          </Button>
        </Gate>
      }
      footer={<ViewsBar views={savedViews} entityLabel="quotes" />}
      toolbar={
        <>
          <ListToolbarSearch
            value={search}
            onChange={setSearch}
            placeholder="Search quotes..."
            className="max-w-xs"
          />
          <BaseFilter
            entries={[
              { label: 'Draft', value: 'DRAFT' },
              { label: 'Pending', value: 'PENDING' },
              { label: 'Accepted', value: 'ACCEPTED' },
              { label: 'Declined', value: 'DECLINED' },
              { label: 'Expired', value: 'EXPIRED' },
              { label: 'Cancelled', value: 'CANCELLED' },
            ]}
            emptyLabel="All Statuses"
            selected={status !== undefined ? [QuoteStatus[status]] : []}
            onSelectionChange={(value, checked) =>
              setStatus(checked ? QuoteStatus[value as keyof typeof QuoteStatus] : undefined)
            }
          />
          <FilterBar
            entityType={CustomPropertyEntityType.QUOTE}
            filters={filters}
            onChange={setFilters}
          />
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={isLoading} onClick={() => quotesQuery.refetch()} />
      }
    >
      <QuotesTable
        data={quotes}
        totalCount={totalCount}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
        isLoading={isLoading}
        columnState={savedViews.columnState}
      />
    </ListPageLayout>
  )
}
