import { PlusIcon } from '@md/icons'
import { Button } from '@md/ui'
import { FileUpIcon } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Link } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { Slot, hasSlot } from '@/edition'
import { MultiFilter } from '@/features/TablePage'
import { useCustomPropertyColumns } from '@/features/customproperties/useCustomPropertyColumns'
import { FilterBar } from '@/features/filters/FilterBar'
import { useEntityFilters } from '@/features/filters/useEntityFilters'
import { useRequestFilters } from '@/features/filters/useRequestFilters'
import { ViewsBar } from '@/features/listviews/ViewsBar'
import { useSavedViews } from '@/features/listviews/useSavedViews'
import { SubscriptionsTable } from '@/features/subscriptions'
import { SubscriptionsImportModal } from '@/features/subscriptions/modals/SubscriptionsImportModal'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { ARRAY_SERDE, useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { EntityListType } from '@/rpc/api/listviews/v1/models_pb'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { SortingState } from '@tanstack/react-table'

const DEFAULT_STATUSES = ['pending', 'trialing', 'active']
const DEFAULT_SORTING: SortingState = []
// URL params captured by saved views as quick filters.
const QUICK_KEYS = ['status']

export const Subscriptions = () => {
  const isExpress = useIsExpressOrganization()
  const [search, setSearch] = useState('')
  const [importVisible, setImportVisible] = useState(false)
  const [sorting, setSorting] = useState<SortingState>(DEFAULT_SORTING)
  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useListPagination()
  const [statusFilter, setStatusFilter] = useQueryState('status', DEFAULT_STATUSES, ARRAY_SERDE)
  const [filters, setFilters] = useEntityFilters()
  const requestFilters = useRequestFilters(CustomPropertyEntityType.SUBSCRIPTION, filters)
  const { hiddenIds: cpHiddenIds } = useCustomPropertyColumns(CustomPropertyEntityType.SUBSCRIPTION)
  const savedViews = useSavedViews({
    listType: EntityListType.SUBSCRIPTION,
    quickKeys: QUICK_KEYS,
    filters,
    sorting,
    setSorting,
    defaultSorting: DEFAULT_SORTING,
    tableKey: 'subscriptions',
    defaultHidden: cpHiddenIds,
  })
  const queryEnabled = savedViews.ready && requestFilters.ready

  useResetPageOnChange(setPagination, [debouncedSearch, statusFilter, filters])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  const subscriptionsQuery = useQuery(
    listSubscriptions,
    {
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
      status: statusFilter.map(mapSubscriptionStatusToGrpc),
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
      orderBy: sortingStateToOrderBy(sorting),
      filters: requestFilters.protoFilters,
    },
    { enabled: queryEnabled }
  )

  const data = subscriptionsQuery.data?.subscriptions ?? []
  const count = Number(subscriptionsQuery.data?.paginationMeta?.totalItems ?? 0)
  const isLoading = subscriptionsQuery.isLoading || !queryEnabled

  return (
    <ListPageLayout
      title="Subscriptions"
      count={count}
      actions={
        isExpress ? undefined : (
          <>
            <Button variant="secondary" size="sm" onClick={() => setImportVisible(true)}>
              <FileUpIcon className="h-4 w-4 mr-2" />
              Import CSV
            </Button>
            <Gate perm="billing:subscriptions:manage">
              <Link to="create">
                <Button variant="primary" size="sm" hasIcon>
                  <PlusIcon size={10} fill="white" /> New subscription
                </Button>
              </Link>
            </Gate>
          </>
        )
      }
      footer={<ViewsBar views={savedViews} entityLabel="subscriptions" />}
      toolbar={
        <>
          <ListToolbarSearch
            value={search}
            onChange={setSearch}
            placeholder="Search subscriptions"
            className="max-w-xs"
          />
          <MultiFilter
            emptyLabel="All statuses"
            entries={[
              { label: 'Pending', value: 'pending' },
              { label: 'Trialing', value: 'trialing' },
              { label: 'Active', value: 'active' },
              { label: 'Canceled', value: 'canceled' },
              { label: 'Aborted', value: 'aborted' },
              { label: 'Ended', value: 'ended' },
              { label: 'Trial Expired', value: 'trial_expired' },
              { label: 'Errored', value: 'errored' },
            ]}
            hook={[statusFilter, setStatusFilter]}
          />
          <FilterBar
            entityType={CustomPropertyEntityType.SUBSCRIPTION}
            filters={filters}
            onChange={setFilters}
          />
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={isLoading} onClick={() => subscriptionsQuery.refetch()} />
      }
    >
      <SubscriptionsTable
        tableKey="subscriptions"
        columnState={savedViews.columnState}
        data={data}
        totalCount={count}
        pagination={pagination}
        setPagination={setPagination}
        isLoading={isLoading}
        sorting={sorting}
        onSortingChange={handleSortingChange}
        chart={
          hasSlot('listPage.metricStrip') ? (
            <Slot name="listPage.metricStrip" metricKey="active_subscriptions" />
          ) : undefined
        }
      />
      <SubscriptionsImportModal
        openState={[importVisible, setImportVisible]}
        onSuccess={() => subscriptionsQuery.refetch()}
      />
    </ListPageLayout>
  )
}

function mapSubscriptionStatusToGrpc(s: string): SubscriptionStatus {
  switch (s) {
    case 'pending':
      return SubscriptionStatus.PENDING
    case 'trialing':
      return SubscriptionStatus.TRIALING
    case 'active':
      return SubscriptionStatus.ACTIVE
    case 'canceled':
      return SubscriptionStatus.CANCELED
    case 'aborted':
      return SubscriptionStatus.ABORTED
    case 'ended':
      return SubscriptionStatus.ENDED
    case 'trial_expired':
      return SubscriptionStatus.TRIAL_EXPIRED
    case 'errored':
      return SubscriptionStatus.ERRORED
    default:
      throw new Error(`Unknown status: ${s}`)
  }
}
