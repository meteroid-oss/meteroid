import { Button } from '@md/ui'
import { useCallback, useState } from 'react'
import { Link } from 'react-router'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { Slot, hasSlot } from '@/edition'
import { useCustomPropertyColumns } from '@/features/customproperties/useCustomPropertyColumns'
import { FilterBar, optionField } from '@/features/filters/FilterBar'
import { ContextChip } from '@/features/filters/FilterChip'
import { useEntityFilters } from '@/features/filters/useEntityFilters'
import { useRequestFilters } from '@/features/filters/useRequestFilters'
import { InvoicesTable } from '@/features/invoices'
import { INVOICES_DEFAULT_HIDDEN } from '@/features/invoices/InvoicesTable'
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

const STATUS_FIELD = optionField(
  'status',
  'Status',
  [
    { label: 'Draft', value: String(InvoiceStatus.DRAFT) },
    { label: 'Finalized', value: String(InvoiceStatus.FINALIZED) },
    { label: 'Uncollectible', value: String(InvoiceStatus.UNCOLLECTIBLE) },
    { label: 'Void', value: String(InvoiceStatus.VOID) },
    { label: 'Closed', value: String(InvoiceStatus.CLOSED) },
  ],
  { multi: false }
)

const PAYMENT_FIELD = optionField(
  'payment_status',
  'Payment',
  [
    { label: 'Unpaid', value: String(InvoicePaymentStatus.UNPAID) },
    { label: 'Processing', value: String(InvoicePaymentStatus.PROCESSING) },
    { label: 'Partially paid', value: String(InvoicePaymentStatus.PARTIALLY_PAID) },
    { label: 'Paid', value: String(InvoicePaymentStatus.PAID) },
    { label: 'Errored', value: String(InvoicePaymentStatus.ERRORED) },
    { label: 'Carried forward', value: String(InvoicePaymentStatus.CARRIED) },
  ],
  { multi: false }
)

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
  // No invoice at all gets the empty state; anything that narrows the list (search, a filter, a
  // view, a subscription's or overdue context) keeps the table, so the filters stay reachable.
  const isNarrowed =
    !!debouncedText ||
    status !== undefined ||
    paymentStatus !== undefined ||
    !!subscription ||
    overdue ||
    filters.length > 0 ||
    !!savedViews.activeView
  const isEmpty =
    !isNarrowed && pagination.pageIndex === 0 && invoicesQuery.isSuccess && count === 0

  return (
    <ListPageLayout
      title="Invoices"
      actions={
        isExpress ? undefined : (
          <>
            <Button variant="secondary" disabled size="sm">
              Import / Export
            </Button>
            <Gate perm="billing:invoices:manage">
              <Link to="create">
                <Button variant="primary" size="sm" hasIcon>
                  <AddIcon /> New invoice
                </Button>
              </Link>
            </Gate>
          </>
        )
      }
      footer={<ViewsBar views={savedViews} entityLabel="invoices" />}
      toolbar={
        <>
          <ListToolbarSearch value={text} onChange={setText} placeholder="Search invoices" />
          <FilterBar
            entityType={CustomPropertyEntityType.INVOICE}
            filters={filters}
            onChange={setFilters}
            local={[
              {
                field: STATUS_FIELD,
                values: status !== undefined ? [String(status)] : [],
                set: ([value]) => setStatus(value ? (Number(value) as InvoiceStatus) : undefined),
              },
              {
                field: PAYMENT_FIELD,
                values: paymentStatus !== undefined ? [String(paymentStatus)] : [],
                set: ([value]) =>
                  setPaymentStatus(value ? (Number(value) as InvoicePaymentStatus) : undefined),
              },
            ]}
            entityGroup="Properties"
          />
          {subscription && (
            <ContextChip
              label="Subscription"
              value={subscription}
              onRemove={() => setSubscription(undefined)}
              removeLabel="Show invoices of every subscription"
            />
          )}
          {overdue && (
            <ContextChip
              label="Overdue"
              onRemove={() => setOverdue(false)}
              removeLabel="Show invoices whether overdue or not"
            />
          )}
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={isLoading} onClick={() => invoicesQuery.refetch()} />
      }
    >
      {isEmpty ? (
        <EmptyState
          title="No invoices yet"
          description="Invoices show up here once a subscription bills, first as drafts and then finalized."
          scene="invoices"
          actions={
            isExpress ? undefined : (
              <Gate perm="billing:invoices:manage">
                <Button size="sm" variant="default" asChild>
                  <Link to="create">New invoice</Link>
                </Button>
              </Gate>
            )
          }
        />
      ) : (
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
      )}
    </ListPageLayout>
  )
}
