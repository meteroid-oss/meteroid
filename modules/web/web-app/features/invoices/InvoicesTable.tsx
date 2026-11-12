import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState, SortingState } from '@tanstack/react-table'
import { CheckCircleIcon, Eye, MoreVerticalIcon, TriangleAlertIcon } from 'lucide-react'
import { ReactNode, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { ColumnState, StandardTable } from '@/components/table/StandardTable'
import {
  useCustomPropertyColumns,
  withCustomPropertyColumns,
} from '@/features/customproperties/useCustomPropertyColumns'
import { InvoicePaymentBadge, InvoiceStatusBadge } from '@/features/invoices/InvoiceStatusBadge'
import { MarkAsPaidDialog } from '@/features/invoices/MarkAsPaidDialog'
import { amountFormat } from '@/features/invoices/amountFormat'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePrefetch } from '@/hooks/usePrefetch'
import { useQuery } from '@/lib/connectrpc'
import { InvoiceConfirmationDialog } from '@/pages/tenants/invoice/InvoiceConfirmationDialog'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import {
  finalizeInvoice,
  getInvoice,
  listInvoices,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import {
  EInvoicingStatus,
  InvoicePaymentStatus,
  InvoiceStatus,
  Invoice,
} from '@/rpc/api/invoices/v1/models_pb'
import { formatDateOnly, formatDateOnlyOptional } from '@/utils/date'

interface InvoicesTableProps {
  data: Invoice[]
  tableKey?: string
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  chart?: ReactNode
  /** Column visibility controlled by an active saved view. */
  columnState?: ColumnState
}

/** Base columns hidden until the user opts in. */
export const INVOICES_DEFAULT_HIDDEN = ['due_date']

const csvCell = (value: string | undefined) => `"${String(value ?? '').replace(/"/g, '""')}"`

const exportInvoicesToCsv = (invoices: Invoice[]) => {
  const headers = ['Invoice number', 'Customer', 'Amount', 'Currency', 'Invoice date']
  const lines = invoices.map(inv =>
    [inv.invoiceNumber, inv.customerName, amountFormat(inv), inv.currency, inv.billingPeriodStart]
      .map(csvCell)
      .join(',')
  )
  const csv = [headers.map(csvCell).join(','), ...lines].join('\n')
  const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url
  link.download = 'invoices.csv'
  link.click()
  URL.revokeObjectURL(url)
}

const InvoiceRowActions = ({ invoiceId }: { invoiceId: string }) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const [shouldFetch, setShouldFetch] = useState(false)
  const [showFinalizeConfirmation, setShowFinalizeConfirmation] = useState(false)
  const [showMarkAsPaidDialog, setShowMarkAsPaidDialog] = useState(false)

  const invoiceQuery = useQuery(getInvoice, shouldFetch ? { id: invoiceId } : skipToken)
  const invoice = invoiceQuery.data?.invoice

  const invalidateList = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listInvoices.parent,
        cardinality: undefined,
      }),
    })

  const invalidateDetail = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: getInvoice,
        input: { id: invoiceId },
        cardinality: 'finite',
      }),
    })

  const finalizeMutation = useMutation(finalizeInvoice, {
    onSuccess: async () => {
      toast.success('Invoice finalized')
      await Promise.all([invalidateDetail(), invalidateList()])
    },
    onError: error => toast.error(`Failed to finalize invoice: ${error.message}`),
  })

  const isConsolidatedChild = !!invoice?.consolidatedIntoInvoiceId
  const canFinalize = invoice?.status === InvoiceStatus.DRAFT && !isConsolidatedChild
  const canMarkAsPaid =
    invoice?.status === InvoiceStatus.FINALIZED &&
    invoice?.paymentStatus !== InvoicePaymentStatus.PAID &&
    Number(invoice?.amountDue) > 0 &&
    !isConsolidatedChild

  const markAsPaidDisabledReason = !invoice
    ? undefined
    : isConsolidatedChild
      ? 'Merged into a consolidated invoice'
      : invoice.status !== InvoiceStatus.FINALIZED
        ? 'Invoice must be finalized first'
        : invoice.paymentStatus === InvoicePaymentStatus.PAID
          ? 'Invoice is already paid'
          : Number(invoice.amountDue) <= 0
            ? 'No amount due'
            : undefined

  const finalizeDisabledReason = !invoice
    ? undefined
    : isConsolidatedChild
      ? 'Merged into a consolidated invoice'
      : invoice.status !== InvoiceStatus.DRAFT
        ? 'Only draft invoices can be finalized'
        : undefined

  return (
    <div onClick={e => e.stopPropagation()}>
      {invoice && (
        <>
          <InvoiceConfirmationDialog
            open={showFinalizeConfirmation}
            onOpenChange={setShowFinalizeConfirmation}
            onConfirm={() => {
              setShowFinalizeConfirmation(false)
              finalizeMutation.mutateAsync({ id: invoiceId })
            }}
            icon={CheckCircleIcon}
            title="Finalize & Send invoice"
            description="Finalize this invoice and send it to the customer. Once finalized, the invoice cannot be edited."
            invoiceNumber={invoice.invoiceNumber}
          />
          <MarkAsPaidDialog
            open={showMarkAsPaidDialog}
            onOpenChange={setShowMarkAsPaidDialog}
            invoiceId={invoiceId}
            invoiceNumber={invoice.invoiceNumber}
            currency={invoice.currency}
            totalAmount={(Number(invoice.amountDue) / 100).toFixed(2)}
          />
        </>
      )}
      <DropdownMenu
        onOpenChange={open => {
          if (open) setShouldFetch(true)
        }}
      >
        <DropdownMenuTrigger asChild>
          <MoreVerticalIcon size={16} className="cursor-pointer" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={() => navigate(`${basePath}/invoices/${invoiceId}`)}>
            <Eye size={16} className="mr-2" />
            View
          </DropdownMenuItem>
          <Tooltip>
            <TooltipTrigger asChild>
              <span>
                <DropdownMenuItem
                  disabled={!invoice || !canMarkAsPaid}
                  onClick={() => setShowMarkAsPaidDialog(true)}
                >
                  <CheckCircleIcon size={16} className="mr-2" />
                  Mark as Paid
                </DropdownMenuItem>
              </span>
            </TooltipTrigger>
            {markAsPaidDisabledReason && (
              <TooltipContent>{markAsPaidDisabledReason}</TooltipContent>
            )}
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <span>
                <DropdownMenuItem
                  disabled={!invoice || !canFinalize}
                  onClick={() => setShowFinalizeConfirmation(true)}
                >
                  <CheckCircleIcon size={16} className="mr-2" />
                  Finalize & Send
                </DropdownMenuItem>
              </span>
            </TooltipTrigger>
            {finalizeDisabledReason && <TooltipContent>{finalizeDisabledReason}</TooltipContent>}
          </Tooltip>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}

export const InvoicesTable = ({
  data,
  tableKey,
  pagination,
  setPagination,
  totalCount,
  isLoading,
  sorting,
  onSortingChange,
  chart,
  columnState,
}: InvoicesTableProps) => {
  const basePath = useBasePath()
  const prefetch = usePrefetch()
  const isExpress = useIsExpressOrganization()
  const [now] = useState(() => new Date())

  const { columns: customPropertyColumns, hiddenIds: customPropertyHiddenIds } =
    useCustomPropertyColumns<Invoice>(CustomPropertyEntityType.INVOICE)

  const baseColumns = useMemo<ColumnDef<Invoice>[]>(
    () => [
      {
        id: 'invoice_number',
        header: 'Invoice Number',
        cell: ({ row }) =>
          row.original.consolidatedIntoInvoiceId
            ? `Merged into ${row.original.consolidatedIntoInvoiceNumber ?? 'consolidated invoice'}`
            : row.original.invoiceNumber,
      },
      {
        id: 'customer_name',
        header: 'Customer',
        accessorKey: 'customerName',
      },
      {
        id: 'amount',
        header: 'Amount',
        accessorFn: amountFormat,
      },
      {
        header: 'Currency',
        accessorKey: 'currency',
        enableSorting: false,
      },
      {
        // The stable "moment" of the invoice (billing period start), not the emission date —
        // and it's what we sort by, so the shown value and the ordering agree.
        id: 'billing_period_start',
        header: 'Invoice date',
        accessorFn: cell =>
          cell.billingPeriodStart ? formatDateOnly(cell.billingPeriodStart) : '—',
      },
      {
        id: 'status',
        header: 'Status',
        enableSorting: true,
        cell: ({ row }) => (
          <div className="flex items-center gap-1.5">
            <InvoiceStatusBadge
              status={row.original.status}
              consolidatedInto={row.original.consolidatedIntoInvoiceId}
            />
            {row.original.einvoicingStatus === EInvoicingStatus.EINVOICING_FAILED && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <TriangleAlertIcon
                    size={14}
                    className="text-amber-600"
                    aria-label="E-invoice not generated"
                  />
                </TooltipTrigger>
                <TooltipContent>
                  Could not be issued as an e-invoice. Open the invoice for the reasons.
                </TooltipContent>
              </Tooltip>
            )}
          </div>
        ),
      },
      {
        id: 'payment_status',
        header: 'Payment Status',
        enableSorting: true,
        // A merged child isn't charged on its own; its payment happens on the consolidated parent.
        cell: ({ row }) =>
          row.original.consolidatedIntoInvoiceId ? (
            <span className="text-muted-foreground">N/A</span>
          ) : (
            <InvoicePaymentBadge invoice={row.original} now={now} />
          ),
      },
      ...(tableKey
        ? [
            {
              id: 'due_date' as const,
              header: 'Due date',
              enableSorting: false,
              // Never-issued invoices (draft, or closed with nothing to bill) carry only an
              // unstamped draft-default due date — show a dash rather than a misleading value.
              // Finalized invoices show their real due date, formatted like the invoice date.
              cell: ({ row }: { row: { original: Invoice } }) =>
                row.original.status === InvoiceStatus.DRAFT ||
                row.original.status === InvoiceStatus.CLOSED
                  ? '—'
                  : (formatDateOnlyOptional(row.original.dueAt) ?? '—'),
            },
          ]
        : []),
      ...(!isExpress
        ? [
            {
              id: 'actions' as const,
              accessorKey: 'id' as const,
              header: '',
              className: 'w-2',
              enableSorting: false,
              cell: ({ row }: { row: { original: Invoice } }) => (
                <Gate perm="billing:invoices:manage">
                  <InvoiceRowActions invoiceId={row.original.id} />
                </Gate>
              ),
            },
          ]
        : []),
    ],
    [isExpress, tableKey, now]
  )

  // Custom-property columns are opt-in via the view menu, which only exists when the table
  // has a `tableKey` (persisted visibility). Skip them on embedded, keyless invoice lists.
  const columns = useMemo(
    () => (tableKey ? withCustomPropertyColumns(baseColumns, customPropertyColumns) : baseColumns),
    [baseColumns, customPropertyColumns, tableKey]
  )

  return (
    <StandardTable
      tableKey={tableKey}
      columnState={columnState}
      columns={columns}
      data={data}
      sortable={true}
      sorting={sorting}
      onSortingChange={onSortingChange}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
      rowLink={row => `${basePath}/invoices/${row.original.id}`}
      onRowIntent={row => prefetch(getInvoice, { id: row.original.id })}
      defaultHidden={
        tableKey ? [...INVOICES_DEFAULT_HIDDEN, ...customPropertyHiddenIds] : undefined
      }
      enableSelection={!!tableKey}
      getRowId={row => row.id}
      renderSelectionActions={rows => (
        <Button
          variant="secondary"
          size="sm"
          onClick={() => exportInvoicesToCsv(rows.map(r => r.original))}
        >
          Export CSV
        </Button>
      )}
      chart={chart}
    />
  )
}
