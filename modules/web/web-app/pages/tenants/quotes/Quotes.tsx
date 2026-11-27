import { Button } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { useCallback, useState } from 'react'
import { Link } from 'react-router'

import { Gate } from '@/components/Gate'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageHeader, ListToolbarSearch, RefreshButton, TableCard } from '@/components/layouts'
import { Slot, hasSlot } from '@/edition'
import { useCustomPropertyColumns } from '@/features/customproperties/useCustomPropertyColumns'
import { FilterBar, optionField } from '@/features/filters/FilterBar'
import { useEntityFilters } from '@/features/filters/useEntityFilters'
import { useRequestFilters } from '@/features/filters/useRequestFilters'
import { ViewsBar } from '@/features/listviews/ViewsBar'
import { useSavedViews } from '@/features/listviews/useSavedViews'
import { QuoteListTabs } from '@/features/quotes/QuoteListTabs'
import { QuotesTable } from '@/features/quotes/QuotesTable'
import { UnsavedQuoteDrafts } from '@/features/quotes/drafts/UnsavedQuoteDrafts'
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

const STATUS_FIELD = optionField(
  'status',
  'Status',
  [
    { label: 'Draft', value: 'DRAFT' },
    { label: 'Pending', value: 'PENDING' },
    { label: 'Accepted', value: 'ACCEPTED' },
    { label: 'Declined', value: 'DECLINED' },
    { label: 'Expired', value: 'EXPIRED' },
    { label: 'Cancelled', value: 'CANCELLED' },
  ],
  { multi: false }
)

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
    <div data-full-bleed className="flex min-h-0 flex-1 flex-col">
      <ListPageHeader
        title="Quotes"
        tabs={<QuoteListTabs />}
        actions={
          <>
            <Slot name="quoteList.actions" />
            <Gate perm="billing:quotes:manage">
              {hasSlot('quoteList.newQuote') ? (
                <Slot name="quoteList.newQuote" />
              ) : (
                <Button asChild variant="primary" size="sm" hasIcon>
                  <Link to={`${basePath}/quotes/create`}>
                    <AddIcon />
                    New Quote
                  </Link>
                </Button>
              )}
            </Gate>
          </>
        }
      />
      <div className="flex shrink-0 flex-col gap-3 px-3 pb-3 empty:hidden">
        <Slot name="quoteList.aside" />
        <UnsavedQuoteDrafts />
      </div>
      <TableCard
        footer={<ViewsBar views={savedViews} entityLabel="quotes" />}
        toolbar={
          <>
            <ListToolbarSearch value={search} onChange={setSearch} placeholder="Search quotes..." />
            <FilterBar
              entityType={CustomPropertyEntityType.QUOTE}
              filters={filters}
              onChange={setFilters}
              local={[
                {
                  field: STATUS_FIELD,
                  values: status !== undefined ? [QuoteStatus[status]] : [],
                  set: ([value]) =>
                    setStatus(value ? QuoteStatus[value as keyof typeof QuoteStatus] : undefined),
                },
              ]}
              entityGroup="Properties"
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
      </TableCard>
    </div>
  )
}
