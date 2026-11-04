import { PlusIcon } from '@md/icons'
import { Badge, Button } from '@md/ui'
import { X } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Link } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { Slot, hasSlot } from '@/edition'
import { useCustomPropertyColumns } from '@/features/customproperties/useCustomPropertyColumns'
import { FilterBar } from '@/features/filters/FilterBar'
import { useEntityFilters } from '@/features/filters/useEntityFilters'
import { useRequestFilters } from '@/features/filters/useRequestFilters'
import { InvoicesTable } from '@/features/invoices'
import { FilterDropdown } from '@/features/invoices/FilterDropdown'
import { INVOICES_DEFAULT_HIDDEN } from '@/features/invoices/InvoicesTable'
import { PaymentFilterDropdown } from '@/features/invoices/PaymentFilterDropdown'
import { ViewsBar } from '@/features/listviews/ViewsBar'
import { useSavedViews } from '@/features/listviews/useSavedViews'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { enumSerde, useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { InvoicePaymentStatus, InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'
import { EntityListType } from '@/rpc/api/listviews/v1/models_pb'

import type { SortingState } from '@tanstack/react-table'

const statusSerde = enumSerde(InvoiceStatus)
const textSerde = {
  serialize: (value: string | undefined) => value ?? '',
  deserialize: (value: string) => value || undefined,
}
const paymentStatusSerde = enumSerde(InvoicePaymentStatus)
const flagSerde = {
  serialize: (value: boolean) => (value ? 'true' : ''),
  deserialize: (value: string) => value === 'true',
}

const DEFAULT_SORTING: SortingState = [{ id: 'billing_period_start', desc: true }]
// URL params captured by saved views as quick filters.
const QUICK_KEYS = ['status', 'payment_status']

export const Invoices = () => {
  const isExpress = useIsExpressOrganization()
  const [text, setText] = useState('')
  const [status, setStatus] = useQueryState<InvoiceStatus | undefined>(
    'status',
    undefined,
    statusSerde
  )
  const [paymentStatus, setPaymentStatus] = useQueryState<InvoicePaymentStatus | undefined>(
    'payment_status',
    undefined,
    paymentStatusSerde
  )
  // Set by a subscription's "All invoices" link.
  const [subscription, setSubscription] = useQueryState<string | undefined>(
    'subscription',
    undefined,
    textSerde
  )
  // Set by a subscription's Outstanding amount.
  const [overdue, setOverdue] = useQueryState<boolean>('overdue', false, flagSerde)
  const [sorting, setSorting] = useState<SortingState>(DEFAULT_SORTING)

  const debouncedText = useDebounceValue(text, 400)

  const [pagination, setPagination] = useListPagination()

  const [filters, setFilters] = useEntityFilters()
  const requestFilters = useRequestFilters(CustomPropertyEntityType.INVOICE, filters)
  const { hiddenIds: cpHiddenIds } = useCustomPropertyColumns(CustomPropertyEntityType.INVOICE)
  const savedViews = useSavedViews({
    listType: EntityListType.INVOICE,
    quickKeys: QUICK_KEYS,
    filters,
    sorting,
    setSorting,
    defaultSorting: DEFAULT_SORTING,
    tableKey: 'invoices',
    defaultHidden: [...INVOICES_DEFAULT_HIDDEN, ...cpHiddenIds],
  })
  const queryEnabled = savedViews.ready && requestFilters.ready

  useResetPageOnChange(setPagination, [
    debouncedText,
    status,
    paymentStatus,
    subscription,
    overdue,
    filters,
  ])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  const invoicesQuery = useQuery(
    listInvoices,
    {
      orderBy: sortingStateToOrderBy(sorting),
      search: debouncedText || '',
      status,
      paymentStatus,
      subscriptionId: subscription,
      overdue: overdue || undefined,
      filters: requestFilters.protoFilters,
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
    },
    { enabled: queryEnabled }
  )

  const data = invoicesQuery.data?.invoices ?? []
  const count = invoicesQuery.data?.paginationMeta?.totalItems ?? 0
  const isLoading = invoicesQuery.isLoading || !queryEnabled

  return (
    <ListPageLayout
      title="Invoices"
      count={count}
      actions={
        isExpress ? undefined : (
          <>
            <Button variant="secondary" disabled size="sm">
              Import / Export
            </Button>
            <Gate perm="billing:invoices:manage">
              <Link to="create">
                <Button variant="primary" size="sm" hasIcon>
                  <PlusIcon size={10} fill="white" /> New invoice
                </Button>
              </Link>
            </Gate>
          </>
        )
      }
      footer={<ViewsBar views={savedViews} entityLabel="invoices" />}
      toolbar={
        <>
          <ListToolbarSearch
            value={text}
            onChange={setText}
            placeholder="Search invoices"
            className="max-w-xs"
          />
          <FilterDropdown status={status} setStatus={setStatus} />
          <PaymentFilterDropdown status={paymentStatus} setStatus={setPaymentStatus} />
          {subscription && (
            <Badge variant="secondary" className="gap-1">
              Subscription {subscription}
              <button
                type="button"
                aria-label="Show invoices of every subscription"
                onClick={() => setSubscription(undefined)}
              >
                <X size={12} />
              </button>
            </Badge>
          )}
          {overdue && (
            <Badge variant="secondary" className="gap-1">
              Overdue
              <button
                type="button"
                aria-label="Show invoices whether overdue or not"
                onClick={() => setOverdue(false)}
              >
                <X size={12} />
              </button>
            </Badge>
          )}
          <FilterBar
            entityType={CustomPropertyEntityType.INVOICE}
            filters={filters}
            onChange={setFilters}
          />
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={isLoading} onClick={() => invoicesQuery.refetch()} />
      }
    >
      <InvoicesTable
        tableKey="invoices"
        data={data}
        totalCount={count}
        pagination={pagination}
        setPagination={setPagination}
        isLoading={isLoading}
        sorting={sorting}
        onSortingChange={handleSortingChange}
        chart={
          hasSlot('listPage.metricStrip') ? (
            <Slot name="listPage.metricStrip" metricKey="outstanding_ar" />
          ) : undefined
        }
        columnState={savedViews.columnState}
      />
    </ListPageLayout>
  )
}
