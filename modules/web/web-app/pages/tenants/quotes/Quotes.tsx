import { disableQuery, useMutation } from '@connectrpc/connect-query'
import { SearchIcon } from '@md/icons'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  InputWithIcon,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Eye, FileText, MoreVerticalIcon, Plus, RefreshCw, Send } from 'lucide-react'
import { FC, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { BaseFilter } from '@/features/TablePage'
import { QuoteStatusBadge } from '@/features/quotes/QuoteStatusBadge'
import { SendQuoteDialog } from '@/features/quotes/SendQuoteDialog'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { Quote, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import {
  cancelQuote,
  getQuote,
  listQuotes,
  sendQuote,
} from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { ListQuotesRequest_SortBy } from '@/rpc/api/quotes/v1/quotes_pb'
import { parseAndFormatDate } from '@/utils/date'

export const Quotes = () => {
  const basePath = useBasePath()

  const [search, setSearch] = useState('')
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const [sortBy, setSortBy] = useState<string>('created_at_desc')

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
    sortBy:
      sortBy === 'created_at_desc'
        ? ListQuotesRequest_SortBy.CREATED_AT_DESC
        : sortBy === 'created_at_asc'
          ? ListQuotesRequest_SortBy.CREATED_AT_ASC
          : sortBy === 'quote_number_desc'
            ? ListQuotesRequest_SortBy.QUOTE_NUMBER_DESC
            : sortBy === 'quote_number_asc'
              ? ListQuotesRequest_SortBy.QUOTE_NUMBER_ASC
              : sortBy === 'expires_at_desc'
                ? ListQuotesRequest_SortBy.EXPIRES_AT_DESC
                : sortBy === 'expires_at_asc'
                  ? ListQuotesRequest_SortBy.EXPIRES_AT_ASC
                  : ListQuotesRequest_SortBy.CREATED_AT_DESC,
    pagination: { page: 0, perPage: 50 },
  })

  const quotes = quotesQuery.data?.quotes || []
  const totalCount = quotesQuery.data?.paginationMeta?.totalItems

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

      {/* Filters */}
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
          <BaseFilter
            entries={[
              { label: 'Newest First', value: 'created_at_desc' },
              { label: 'Oldest First', value: 'created_at_asc' },
              { label: 'Quote # (Z-A)', value: 'quote_number_desc' },
              { label: 'Quote # (A-Z)', value: 'quote_number_asc' },
              { label: 'Expires (Soonest)', value: 'expires_at_desc' },
              { label: 'Expires (Latest)', value: 'expires_at_asc' },
            ]}
            emptyLabel="Newest First"
            selected={[sortBy]}
            onSelectionChange={(value, checked) => setSortBy(checked ? value : 'created_at_desc')}
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

      {/* Quotes Table */}
      <div className="border rounded-lg">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Quote</TableHead>
              <TableHead>Customer</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Type</TableHead>
              <TableHead>Created</TableHead>
              <TableHead>Expires</TableHead>
              <TableHead className="w-[100px]">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {quotes.map(quote => (
              <QuoteRow key={quote.id} quote={quote} basePath={basePath} />
            ))}
            {quotes.length === 0 && (
              <TableRow>
                <TableCell colSpan={7} className="text-center py-8 text-muted-foreground">
                  {search || statusFilter !== 'all'
                    ? 'No quotes match your filters'
                    : 'No quotes created yet'}
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </div>

      {/* Pagination could go here */}
    </div>
  )
}

interface QuoteRowProps {
  quote: Quote
  basePath: string
}

const QuoteRow: FC<QuoteRowProps> = ({ quote, basePath }) => {
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
    <>
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

      <TableRow>
        <TableCell>
          <div className="font-medium">
            <Link to={`${basePath}/quotes/${quote.id}`} className="text-brand hover:underline">
              {quote.quoteNumber}
            </Link>
          </div>
        </TableCell>
        <TableCell>
          <div className="font-medium">{quote.customerName || 'Customer'}</div>
          <div className="text-sm text-muted-foreground">{quote.customerId}</div>
        </TableCell>
        <TableCell>
          <QuoteStatusBadge status={quote.status} />
        </TableCell>

        <TableCell>
          <div className="font-medium">Subscription Quote</div>
          <div className="text-sm text-muted-foreground">{quote.currency}</div>
        </TableCell>
        <TableCell className="text-muted-foreground">
          {quote.createdAt ? parseAndFormatDate(quote.createdAt) : '—'}
        </TableCell>
        <TableCell className="text-muted-foreground">
          {quote.expiresAt ? parseAndFormatDate(quote.expiresAt) : '—'}
        </TableCell>
        <TableCell>
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
        </TableCell>
      </TableRow>
    </>
  )
}

