import { PlusIcon } from '@md/icons'
import { Button, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { FileUpIcon } from 'lucide-react'
import { Fragment, FunctionComponent, useCallback, useState } from 'react'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import {
  ListPageLayout,
  ListToolbarSearch,
  RefreshButton,
  TenantPageLayout,
} from '@/components/layouts'
import { MemberPicker } from '@/features/accounts/components/MemberPicker'
import { useAssignOwner } from '@/features/accounts/hooks/useAssignOwner'
import { useMembers } from '@/features/accounts/hooks/useMembers'
import { CustomersCreatePanel, CustomersTable } from '@/features/customers'
import { CustomersExportModal } from '@/features/customers/modals/CustomersExportModal'
import { CustomersImportModal } from '@/features/customers/modals/CustomersImportModal'
import {
  ArchivedScope,
  CustomerStatusFilter,
} from '@/features/customers/table/CustomerStatusFilter'
import { useCustomPropertyColumns } from '@/features/customproperties/useCustomPropertyColumns'
import { FilterBar } from '@/features/filters/FilterBar'
import { AppFilter } from '@/features/filters/filterTypes'
import { useEntityFilters } from '@/features/filters/useEntityFilters'
import { useRequestFilters } from '@/features/filters/useRequestFilters'
import { ViewsBar } from '@/features/listviews/ViewsBar'
import { useSavedViews } from '@/features/listviews/useSavedViews'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { ARRAY_SERDE, useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { plural } from '@/lib/utils/format'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { EntityListType } from '@/rpc/api/listviews/v1/models_pb'

import type { CustomerBrief } from '@/rpc/api/customers/v1/models_pb'
import type { Row, SortingState } from '@tanstack/react-table'

// Stable default reference: useQueryState memoizes its value on the default, so an
// inline [] would produce a new array every render and thrash effect dependencies.
const EMPTY_STRING_ARRAY: string[] = []
const DEFAULT_SORTING: SortingState = [{ id: 'name', desc: false }]
// URL params captured by saved views as quick filters.
const QUICK_KEYS = ['records', 'lifecycle']
// The country quick filter became the `country` filter field; old links and views still carry it.
const LEGACY_QUICK = {
  country: (raw: string): AppFilter => ({ field: 'country', op: 'in', values: raw.split(',') }),
}

export const Customers: FunctionComponent = () => {
  const isExpress = useIsExpressOrganization()
  const [createPanelVisible, setCreatePanelVisible] = useState(false)
  const [exportVisible, setExportVisible] = useState(false)
  const [importVisible, setImportVisible] = useState(false)
  const [search, setSearch] = useState('')
  const [sorting, setSorting] = useState<SortingState>(DEFAULT_SORTING)

  const [records, setRecords] = useQueryState<ArchivedScope>('records', 'active')
  const [lifecycle, setLifecycle] = useQueryState<string[]>(
    'lifecycle',
    EMPTY_STRING_ARRAY,
    ARRAY_SERDE
  )
  const [filters, setFilters] = useEntityFilters()
  const requestFilters = useRequestFilters(CustomPropertyEntityType.CUSTOMER, filters)
  const { hiddenIds: cpHiddenIds } = useCustomPropertyColumns(CustomPropertyEntityType.CUSTOMER)
  const savedViews = useSavedViews({
    listType: EntityListType.CUSTOMER,
    quickKeys: QUICK_KEYS,
    filters,
    sorting,
    setSorting,
    defaultSorting: DEFAULT_SORTING,
    tableKey: 'customers',
    defaultHidden: cpHiddenIds,
    legacyQuick: LEGACY_QUICK,
  })
  const queryEnabled = savedViews.ready && requestFilters.ready
  const canAssignOwner = usePermissions().has('customers:manage') && !isExpress

  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useListPagination()

  useResetPageOnChange(setPagination, [debouncedSearch, records, lifecycle, filters])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  // Archived scope is orthogonal to lifecycle: active -> false, archived -> true, all -> undefined.
  const archivedFilter = records === 'archived' ? true : records === 'active' ? false : undefined

  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
      orderBy: sortingStateToOrderBy(sorting),
      archived: archivedFilter,
      lifecycleState: lifecycle,
      filters: requestFilters.protoFilters,
    },
    { enabled: queryEnabled }
  )

  const data = customersQuery.data?.customers ?? []
  const count = customersQuery.data?.paginationMeta?.totalItems ?? 0
  const isLoading = customersQuery.isLoading || !queryEnabled
  // Only the truly empty tenant gets the onboarding empty state; a narrowed list shows an empty
  // table so the views bar and filters stay reachable. Past page 1 the total reads 0, so keep the
  // table mounted there and let it step back to a page with rows.
  const isNarrowed =
    debouncedSearch.length > 0 || !!savedViews.activeView || savedViews.isCustomized
  const isEmpty = count === 0 && !isNarrowed && pagination.pageIndex === 0

  return (
    <Fragment>
      <TenantPageLayout>
        <ListPageLayout
          title="Customers"
          count={count}
          actions={
            isExpress ? undefined : (
              <>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
                      <Button size="sm" variant="secondary" disabled>
                        Export
                      </Button>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>Coming soon</TooltipContent>
                </Tooltip>
                <Button variant="secondary" size="sm" onClick={() => setImportVisible(true)}>
                  <FileUpIcon className="h-4 w-4 mr-2" />
                  Import CSV
                </Button>
                <Gate perm="customers:manage">
                  <Button
                    size="sm"
                    variant="default"
                    hasIcon
                    onClick={() => setCreatePanelVisible(true)}
                  >
                    <PlusIcon className="w-4 h-4" /> New customer
                  </Button>
                </Gate>
              </>
            )
          }
          footer={<ViewsBar views={savedViews} entityLabel="customers" />}
          toolbar={
            <>
              <ListToolbarSearch value={search} onChange={setSearch} className="max-w-xs" />
              <CustomerStatusFilter
                archived={records}
                onArchivedChange={setRecords}
                lifecycle={lifecycle}
                onLifecycleChange={setLifecycle}
              />
              <FilterBar
                entityType={CustomPropertyEntityType.CUSTOMER}
                filters={filters}
                onChange={setFilters}
              />
            </>
          }
          toolbarTrailing={
            <RefreshButton isLoading={isLoading} onClick={() => customersQuery.refetch()} />
          }
        >
          {!isLoading && isEmpty ? (
            <EmptyState
              title="No customers yet"
              description="Create your first customers and assign a subscription"
              imageName="customers"
              actions={
                !isExpress ? (
                  <Gate perm="customers:manage">
                    <Button size="sm" variant="default" onClick={() => setCreatePanelVisible(true)}>
                      New customer
                    </Button>
                  </Gate>
                ) : undefined
              }
            />
          ) : (
            <CustomersTable
              data={data}
              totalCount={count}
              pagination={pagination}
              setPagination={setPagination}
              isLoading={isLoading}
              sorting={sorting}
              onSortingChange={handleSortingChange}
              columnState={savedViews.columnState}
              renderSelectionActions={
                canAssignOwner ? rows => <BulkOwnerActions rows={rows} /> : undefined
              }
            />
          )}
        </ListPageLayout>
      </TenantPageLayout>
      <CustomersExportModal openState={[exportVisible, setExportVisible]} />
      <CustomersImportModal
        openState={[importVisible, setImportVisible]}
        onSuccess={() => customersQuery.refetch()}
      />
      {!isExpress && (
        <CustomersCreatePanel
          visible={createPanelVisible}
          closePanel={() => setCreatePanelVisible(false)}
        />
      )}
    </Fragment>
  )
}

/** "Assign owner" over the selected customers (explicit selection only). */
const BulkOwnerActions = ({ rows }: { rows: Row<CustomerBrief>[] }) => {
  const assign = useAssignOwner()
  const { byId } = useMembers()
  const customerIds = rows.map(r => r.original.id)
  const accounts = plural(customerIds.length, 'customer')

  const onAssign = (ownerUserId: string | undefined) =>
    assign.mutate(
      { customerIds, ownerUserId },
      {
        onSuccess: res =>
          toast.success(
            ownerUserId
              ? `${byId.get(ownerUserId)?.name ?? 'The member'} now owns ${plural(res.updated, 'customer')}`
              : `Removed the owner of ${plural(res.updated, 'customer')}`
          ),
      }
    )

  return (
    <>
      <MemberPicker
        label={`Assign an owner to ${accounts}`}
        placeholder="Assign owner"
        onChange={onAssign}
        disabled={assign.isPending}
        className="border border-border"
      />
      <Button
        variant="ghost"
        size="sm"
        disabled={assign.isPending}
        onClick={() => onAssign(undefined)}
      >
        Remove owner
      </Button>
    </>
  )
}
