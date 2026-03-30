import { disableQuery, useMutation } from '@connectrpc/connect-query'
import { SearchIcon } from '@md/icons'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  InputWithIcon,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef, PaginationState, SortingState } from '@tanstack/react-table'
import { Eye, FileText, MoreVerticalIcon, Plus, RefreshCw, Send } from 'lucide-react'
import { FC, useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { StandardTable } from '@/components/table/StandardTable'
import { BaseFilter } from '@/features/TablePage'
import { QuoteStatusBadge } from '@/features/quotes/QuoteStatusBadge'
import { SendQuoteDialog } from '@/features/quotes/SendQuoteDialog'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { Quote, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import {
  cancelQuote,
  getQuote,
  listQuotes,
  sendQuote,
} from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { parseAndFormatDate, parseAndFormatDateOptional } from '@/utils/date'

export const Quotes = () => {
  const basePath = useBasePath()

  const [search, setSearch] = useState('')
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const [sorting, setSorting] = useState<SortingState>([{ id: 'created_at', desc: true }])
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [search, statusFilter])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    []
  )

  const quotesQuery = useQuery(listQuotes, {
    search: search || undefined,
    status:
      statusFilter !== 'all'
        ? statusFilter === 'DRAFT'
          ? QuoteStatus.DRAFT
          : statusFilter === 'PENDING'
            ? QuoteStatus.PENDING
            : statusFilter === 'ACCEPTED'
              ? QuoteStatus.ACCEPTED
              : statusFilter === 'DECLINED'
                ? QuoteStatus.DECLINED
                : statusFilter === 'EXPIRED'
                  ? QuoteStatus.EXPIRED
                  : statusFilter === 'CANCELLED'
                    ? QuoteStatus.CANCELLED
                    : undefined
        : undefined,
    orderBy: sortingStateToOrderBy(sorting),
    pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
  })

  const quotes = quotesQuery.data?.quotes || []
  const totalCount = Number(quotesQuery.data?.paginationMeta?.totalItems ?? 0)

  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-row items-center justify-between">
        <h1 className="text-2xl font-bold">
          Quotes{' '}
          {totalCount !== undefined && (
            <span className="text-xs font-medium text-muted-foreground">({totalCount})</span>
          )}
        </h1>
        <Button asChild variant="primary" size="sm" hasIcon>
          <Link to={`${basePath}/quotes/create`}>
            <Plus className="w-4 h-4" />
            New Quote
          </Link>
        </Button>
      </div>

      <div className="flex items-center justify-between">
        <div className="flex gap-2 items-center">
          <InputWithIcon
            placeholder="Search quotes..."
            icon={<SearchIcon size={16} />}
            width="fit-content"
            value={search}
            onChange={e => setSearch(e.target.value)}
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
            selected={statusFilter !== 'all' ? [statusFilter] : []}
            onSelectionChange={(value, checked) => setStatusFilter(checked ? value : 'all')}
          />
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={quotesQuery.isLoading}
          onClick={() => quotesQuery.refetch()}
        >
          <RefreshCw size={14} className={quotesQuery.isLoading ? 'animate-spin' : ''} />
        </Button>
      </div>

      <QuotesTable
        data={quotes}
        totalCount={totalCount}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
        isLoading={quotesQuery.isLoading}
      />
    </div>
  )
}

interface QuotesTableProps {
  data: Quote[]
  totalCount: number
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
  sorting: SortingState
  onSortingChange: (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => void
  isLoading: boolean
}

const QuotesTable: FC<QuotesTableProps> = ({
  data,
  totalCount,
  pagination,
  setPagination,
  sorting,
  onSortingChange,
  isLoading,
}) => {
  const basePath = useBasePath()

  const columns = useMemo<ColumnDef<Quote>[]>(
    () => [
      {
        id: 'quote_number',
        header: 'Quote',
        cell: ({ row }) => (
          <span className="font-medium">{row.original.quoteNumber}</span>
        ),
      },
      {
        id: 'customer_name',
        header: 'Customer',
        cell: ({ row }) => <span>{row.original.customerName || 'Customer'}</span>,
      },
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
        id: 'actions',
        className: 'w-[50px]',
        enableSorting: false,
        cell: ({ row }) => <QuoteRowActions quote={row.original} basePath={basePath} />,
      },
    ],
    [basePath]
  )

  return (
    <StandardTable
      columns={columns}
      data={data}
      sortable={true}
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

  const quoteDetailQuery = useQuery(getQuote, showSendDialog ? { id: quote.id } : disableQuery)
  const recipients = quoteDetailQuery.data?.quote?.quote?.recipients

  const sendQuoteMutation = useMutation(sendQuote, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listQuotes.service.typeName] })
    },
  })

  const cancelQuoteMutation = useMutation(cancelQuote, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listQuotes.service.typeName] })
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
