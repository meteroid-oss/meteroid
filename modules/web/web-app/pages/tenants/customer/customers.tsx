import { Button, Tabs, TabsList, TabsTrigger } from '@md/ui'
import { Fragment, FunctionComponent, useCallback, useEffect, useState } from 'react'
import { useSearchParams } from 'react-router'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
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
import { ArchivedScope } from '@/features/customers/table/CustomerStatusFilter'
import { useCustomPropertyColumns } from '@/features/customproperties/useCustomPropertyColumns'
import { FilterBar, optionField } from '@/features/filters/FilterBar'
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
import { LIFECYCLE_BUCKETS, lifecycleStateLabel } from '@/lib/lifecycle'
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

const RECORDS_FIELD = optionField(
  'records',
  'Records',
  [
    { label: 'Not archived', value: 'active' },
    { label: 'Archived', value: 'archived' },
    { label: 'All', value: 'all' },
  ],
  { multi: false }
)

const LIFECYCLE_FIELD = optionField(
  'lifecycle',
  'Lifecycle',
  LIFECYCLE_BUCKETS.flatMap(bucket =>
    bucket.states.map(state => ({
      value: state,
      label: lifecycleStateLabel(state),
      group: bucket.label,
    }))
  )
)
// The country quick filter became the `country` filter field; old links and views still carry it.
const LEGACY_QUICK = {
  country: (raw: string): AppFilter => ({ field: 'country', op: 'in', values: raw.split(',') }),
}

// The header tabs are shortcuts over the Status filter: "All" is the default (every customer that
// isn't archived), Active and Inactive pick a lifecycle bucket, Archived switches the records scope.
// A Status combination none of them covers leaves no tab selected.
type CustomerTab = 'all' | 'active' | 'inactive' | 'archived'
const bucketStates = (key: string) => LIFECYCLE_BUCKETS.find(b => b.key === key)?.states ?? []
const TAB_FILTERS: Record<CustomerTab, { records: ArchivedScope; lifecycle: string[] }> = {
  all: { records: 'active', lifecycle: [] },
  active: { records: 'active', lifecycle: bucketStates('active') },
  inactive: { records: 'active', lifecycle: bucketStates('churned') },
  archived: { records: 'archived', lifecycle: [] },
}
const sameStates = (a: string[], b: string[]) =>
  a.length === b.length && a.every(state => b.includes(state))
const tabFor = (records: ArchivedScope, lifecycle: string[]): CustomerTab | '' =>
  (Object.keys(TAB_FILTERS) as CustomerTab[]).find(
    tab => TAB_FILTERS[tab].records === records && sameStates(TAB_FILTERS[tab].lifecycle, lifecycle)
  ) ?? ''

export const Customers: FunctionComponent = () => {
  const isExpress = useIsExpressOrganization()
  const [createPanelVisible, setCreatePanelVisible] = useState(false)
  // `?create=1` (the nav's New › Customer) opens the create panel, then leaves the URL.
  const [searchParams, setSearchParams] = useSearchParams()
  useEffect(() => {
    if (!searchParams.has('create')) return
    setCreatePanelVisible(true)
    setSearchParams(
      params => {
        params.delete('create')
        return params
      },
      { replace: true }
    )
  }, [searchParams, setSearchParams])
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
          tabs={
            <Tabs
              value={tabFor(records, lifecycle)}
              onValueChange={value => {
                const tab = TAB_FILTERS[value as CustomerTab]
                setRecords(tab.records)
                setLifecycle(tab.lifecycle)
              }}
            >
              <TabsList variant="ghost">
                <TabsTrigger value="all">All</TabsTrigger>
                <TabsTrigger value="active">Active</TabsTrigger>
                <TabsTrigger value="inactive">Inactive</TabsTrigger>
                <TabsTrigger value="archived">Archived</TabsTrigger>
              </TabsList>
            </Tabs>
          }
          actions={
            isExpress ? undefined : (
              <>
                <Button variant="outline" size="sm" onClick={() => setImportVisible(true)}>
                  Import
                </Button>
                <Gate perm="customers:manage">
                  <Button
                    size="sm"
                    variant="primary"
                    hasIcon
                    onClick={() => setCreatePanelVisible(true)}
                  >
                    <AddIcon />
                    Customer
                  </Button>
                </Gate>
              </>
            )
          }
          footer={<ViewsBar views={savedViews} entityLabel="customers" />}
          toolbar={
            <>
              <ListToolbarSearch value={search} onChange={setSearch} />
              <FilterBar
                entityType={CustomPropertyEntityType.CUSTOMER}
                filters={filters}
                onChange={setFilters}
                local={[
                  {
                    // Not archived is the default scope: a chip only for another.
                    field: RECORDS_FIELD,
                    values: records === 'active' ? [] : [records],
                    set: ([value]) => setRecords((value as ArchivedScope) ?? 'active'),
                  },
                  { field: LIFECYCLE_FIELD, values: lifecycle, set: setLifecycle },
                ]}
                entityGroup="Properties"
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
              description="Customers you add or import show up here, together with their subscriptions and invoices."
              scene="customers"
              actions={
                !isExpress ? (
                  // The header's two actions, Import beside New customer, gated the same way.
                  <div className="flex items-center gap-2">
                    <Button size="sm" variant="secondary" onClick={() => setImportVisible(true)}>
                      Import
                    </Button>
                    <Gate perm="customers:manage">
                      <Button
                        size="sm"
                        variant="default"
                        onClick={() => setCreatePanelVisible(true)}
                      >
                        New customer
                      </Button>
                    </Gate>
                  </div>
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
        variant="secondary"
        className="h-7 gap-1.5 px-2.5 font-medium"
      />
      <Button
        variant="secondary"
        size="sm"
        disabled={assign.isPending}
        onClick={() => onAssign(undefined)}
      >
        Remove owner
      </Button>
    </>
  )
}
