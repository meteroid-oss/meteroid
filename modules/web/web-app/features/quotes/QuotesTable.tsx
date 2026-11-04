import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef, PaginationState, SortingState } from '@tanstack/react-table'
import { CopyPlus, Eye, FileText, MoreVerticalIcon, Pencil, Send } from 'lucide-react'
import { FC, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { ColumnState, StandardTable } from '@/components/table/StandardTable'
import {
  ACTIONS_COLUMN_ID,
  useCustomPropertyColumns,
  withCustomPropertyColumns,
} from '@/features/customproperties/useCustomPropertyColumns'
import { QuoteStatusBadge } from '@/features/quotes/QuoteStatusBadge'
import { SendQuoteDialog } from '@/features/quotes/SendQuoteDialog'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { Quote, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import {
  cancelQuote,
  getQuote,
  listQuotes,
  sendQuote,
} from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { parseAndFormatDate, parseAndFormatDateOptional } from '@/utils/date'

export interface QuotesTableProps {
  data: Quote[]
  totalCount: number
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
  sorting?: SortingState
  onSortingChange?: (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => void
  isLoading: boolean
  hideCustomer?: boolean
  /** Column visibility controlled by an active saved view. */
  columnState?: ColumnState
}

export const QuotesTable: FC<QuotesTableProps> = ({
  data,
  totalCount,
  pagination,
  setPagination,
  sorting,
  onSortingChange,
  isLoading,
  columnState,
  hideCustomer,
}) => {
  const basePath = useBasePath()

  const { columns: customPropertyColumns, hiddenIds: customPropertyHiddenIds } =
    useCustomPropertyColumns<Quote>(CustomPropertyEntityType.QUOTE)

  const baseColumns = useMemo<ColumnDef<Quote>[]>(
    () => [
      {
        id: 'quote_number',
        header: 'Quote',
        cell: ({ row }) => <span className="font-medium">{row.original.quoteNumber}</span>,
      },
      ...(hideCustomer
        ? []
        : [
            {
              id: 'customer_name',
              header: 'Customer',
              cell: ({ row }) => <span>{row.original.customerName || 'Customer'}</span>,
            } satisfies ColumnDef<Quote>,
          ]),
      {
        id: 'status',
        header: 'Status',
        cell: ({ row }) => <QuoteStatusBadge status={row.original.status} />,
      },
      {
        header: 'Type',
        enableSorting: false,
        cell: ({ row }) => (
          <div>
            <span className="font-medium">Subscription Quote</span>
            <span className="block text-xs text-muted-foreground">{row.original.currency}</span>
          </div>
        ),
      },
      {
        id: 'created_at',
        header: 'Created',
        cell: ({ row }) => (
          <span className="text-muted-foreground">
            {row.original.createdAt ? parseAndFormatDate(row.original.createdAt) : '—'}
          </span>
        ),
      },
      {
        id: 'expires_at',
        header: 'Expires',
        cell: ({ row }) => (
          <span className="text-muted-foreground">
            {parseAndFormatDateOptional(row.original.expiresAt)}
          </span>
        ),
      },
      {
        header: '',
        id: ACTIONS_COLUMN_ID,
        className: 'w-[50px]',
        enableSorting: false,
        cell: ({ row }) => <QuoteRowActions quote={row.original} basePath={basePath} />,
      },
    ],
    [basePath, hideCustomer]
  )

  const columns = useMemo(
    () => withCustomPropertyColumns(baseColumns, customPropertyColumns),
    [baseColumns, customPropertyColumns]
  )

  return (
    <StandardTable
      tableKey="quotes"
      columnState={columnState}
      defaultHidden={customPropertyHiddenIds}
      columns={columns}
      data={data}
      sortable={!!onSortingChange}
      sorting={sorting}
      onSortingChange={onSortingChange}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
      rowLink={row => `${basePath}/quotes/${row.original.id}`}
    />
  )
}

const QuoteRowActions: FC<{ quote: Quote; basePath: string }> = ({ quote, basePath }) => {
  const queryClient = useQueryClient()
  const [showSendDialog, setShowSendDialog] = useState(false)
  const [customMessage, setCustomMessage] = useState('')

  const quoteDetailQuery = useQuery(getQuote, showSendDialog ? { id: quote.id } : skipToken)
  const recipients = quoteDetailQuery.data?.quote?.quote?.recipients
  const emailEnabled = quoteDetailQuery.data?.quote?.quoteEmailEnabled

  const sendQuoteMutation = useMutation(sendQuote, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listQuotes.parent,
          cardinality: undefined,
        }),
      })
    },
  })

  const cancelQuoteMutation = useMutation(cancelQuote, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listQuotes.parent,
          cardinality: undefined,
        }),
      })
    },
  })

  const handleSendQuote = async () => {
    try {
      await sendQuoteMutation.mutateAsync({ id: quote.id, message: customMessage || undefined })
      toast.success('Quote sent successfully')
      setShowSendDialog(false)
      setCustomMessage('')
    } catch (error) {
      toast.error('Failed to send quote')
    }
  }

  const handleCancelQuote = async (quoteId: string) => {
    try {
      await cancelQuoteMutation.mutateAsync({ id: quoteId })
      toast.success('Quote cancelled')
    } catch (error) {
      toast.error('Failed to cancel quote')
    }
  }

  return (
    <div onClick={e => e.stopPropagation()}>
      <SendQuoteDialog
        open={showSendDialog}
        onOpenChange={open => {
          setShowSendDialog(open)
          if (!open) setCustomMessage('')
        }}
        quoteNumber={quote.quoteNumber}
        recipients={recipients}
        customMessage={customMessage}
        onCustomMessageChange={setCustomMessage}
        onConfirm={handleSendQuote}
        isPending={sendQuoteMutation.isPending}
        emailEnabled={emailEnabled}
      />

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <MoreVerticalIcon size={16} className="cursor-pointer" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem asChild>
            <Link to={`${basePath}/quotes/${quote.id}`}>
              <Eye className="w-4 h-4 mr-2" />
              View
            </Link>
          </DropdownMenuItem>
          {(quote.status === QuoteStatus.DRAFT || quote.status === QuoteStatus.PENDING) && (
            <DropdownMenuItem asChild>
              <Link to={`${basePath}/quotes/${quote.id}/edit`}>
                <Pencil className="w-4 h-4 mr-2" />
                Edit
              </Link>
            </DropdownMenuItem>
          )}
          <DropdownMenuItem asChild>
            <Link to={`${basePath}/quotes/create?from=${quote.id}`}>
              <CopyPlus className="w-4 h-4 mr-2" />
              Use as template
            </Link>
          </DropdownMenuItem>
          {(quote.status === QuoteStatus.DRAFT || quote.status === QuoteStatus.PENDING) && (
            <DropdownMenuItem onClick={() => setShowSendDialog(true)}>
              <Send className="w-4 h-4 mr-2" />
              Send to Customer
            </DropdownMenuItem>
          )}
          {quote.status === QuoteStatus.ACCEPTED && (
            <DropdownMenuItem asChild>
              <Link to={`${basePath}/quotes/${quote.id}/convert`}>
                <FileText className="w-4 h-4 mr-2" />
                Convert to Subscription
              </Link>
            </DropdownMenuItem>
          )}
          {(quote.status === QuoteStatus.DRAFT || quote.status === QuoteStatus.PENDING) && (
            <DropdownMenuItem
              onClick={() => handleCancelQuote(quote.id)}
              className="text-destructive"
            >
              Cancel Quote
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
