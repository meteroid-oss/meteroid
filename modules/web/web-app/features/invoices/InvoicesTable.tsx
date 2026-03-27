import { createConnectQueryKey, disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { CheckCircleIcon, Eye, MoreVerticalIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { StandardTable } from '@/components/table/StandardTable'
import { InvoiceStatusBadge } from '@/features/invoices/InvoiceStatusBadge'
import { MarkAsPaidDialog } from '@/features/invoices/MarkAsPaidDialog'
import { PaymentStatusBadge } from '@/features/invoices/PaymentStatusBadge'
import { amountFormat } from '@/features/invoices/amountFormat'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useQuery } from '@/lib/connectrpc'
import { InvoiceConfirmationDialog } from '@/pages/tenants/invoice/InvoiceConfirmationDialog'
import {
  finalizeInvoice,
  getInvoice,
  listInvoices,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { InvoicePaymentStatus, InvoiceStatus, Invoice } from '@/rpc/api/invoices/v1/models_pb'

interface CustomersTableProps {
  data: Invoice[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  linkPrefix?: string
}

const InvoiceRowActions = ({ invoiceId }: { invoiceId: string }) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const [shouldFetch, setShouldFetch] = useState(false)
  const [showFinalizeConfirmation, setShowFinalizeConfirmation] = useState(false)
  const [showMarkAsPaidDialog, setShowMarkAsPaidDialog] = useState(false)

  const invoiceQuery = useQuery(getInvoice, shouldFetch ? { id: invoiceId } : disableQuery)
  const invoice = invoiceQuery.data?.invoice

  const invalidateList = () =>
    queryClient.invalidateQueries({ queryKey: [listInvoices.service.typeName] })

  const invalidateDetail = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey(getInvoice, { id: invoiceId }),
    })

  const finalizeMutation = useMutation(finalizeInvoice, {
    onSuccess: async () => {
      toast.success('Invoice finalized')
      await Promise.all([invalidateDetail(), invalidateList()])
    },
    onError: error => toast.error(`Failed to finalize invoice: ${error.message}`),
  })

  const canFinalize = invoice?.status === InvoiceStatus.DRAFT
  const canMarkAsPaid =
    invoice?.status === InvoiceStatus.FINALIZED &&
    invoice?.paymentStatus !== InvoicePaymentStatus.PAID &&
    Number(invoice?.amountDue) > 0

  const markAsPaidDisabledReason = !invoice
    ? undefined
    : invoice.status !== InvoiceStatus.FINALIZED
      ? 'Invoice must be finalized first'
      : invoice.paymentStatus === InvoicePaymentStatus.PAID
        ? 'Invoice is already paid'
        : Number(invoice.amountDue) <= 0
          ? 'No amount due'
          : undefined

  const finalizeDisabledReason = !invoice
    ? undefined
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
      <DropdownMenu onOpenChange={open => { if (open) setShouldFetch(true) }}>
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
            {finalizeDisabledReason && (
              <TooltipContent>{finalizeDisabledReason}</TooltipContent>
            )}
          </Tooltip>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}

export const InvoicesTable = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
}: CustomersTableProps) => {
  const basePath = useBasePath()
  const isExpress = useIsExpressOrganization()

  const columns = useMemo<ColumnDef<Invoice>[]>(
    () => [
      {
        header: 'Invoice Number',
        accessorKey: 'invoiceNumber',
      },
      {
        header: 'Customer',
        accessorKey: 'customerName',
      },
      {
        header: 'Amount',
        accessorFn: amountFormat,
      },
      {
        header: 'Currency',
        accessorKey: 'currency',
      },
      {
        header: 'Invoice date',
        accessorFn: cell => cell.invoiceDate,
      },
      {
        header: 'Status',
        cell: ({ row }) => <InvoiceStatusBadge status={row.original.status} />,
      },
      {
        header: 'Payment Status',
        cell: ({ row }) => <PaymentStatusBadge status={row.original.paymentStatus} />,
      },
      ...(!isExpress
        ? [
            {
              accessorKey: 'id' as const,
              header: '',
              className: 'w-2',
              cell: ({ row }: { row: { original: Invoice } }) => (
                <InvoiceRowActions invoiceId={row.original.id} />
              ),
            },
          ]
        : []),
    ],
    [basePath, isExpress]
  )

  return (
    <StandardTable
      columns={columns}
      data={data}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
      rowLink={row => `${basePath}/invoices/${row.original.id}`}
    />
  )
}
