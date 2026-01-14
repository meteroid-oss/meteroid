import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ChevronDown, Edit, Eye, FileText, Plus, Send } from 'lucide-react'
import { FC, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { Quote, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import { listQuotes, sendQuote } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
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

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex justify-between items-center">
        <div>
          <h1 className="text-2xl font-semibold">Quotes</h1>
          <p className="text-muted-foreground">Manage and track your sales quotes</p>
        </div>
        <Button asChild>
          <Link to={`${basePath}/quotes/create`}>
            <Plus className="w-4 h-4 mr-2" />
            Create Quote
          </Link>
        </Button>
      </div>

      {/* Filters */}
      <div className="flex gap-4 items-center">
        <div className="flex-1 max-w-sm">
          <Input
            placeholder="Search quotes..."
            value={search}
            onChange={e => setSearch(e.target.value)}
          />
        </div>
        <Select value={statusFilter} onValueChange={setStatusFilter}>
          <SelectTrigger className="w-[180px]">
            <SelectValue placeholder="Filter by status" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All Statuses</SelectItem>
            <SelectItem value="DRAFT">Draft</SelectItem>
            <SelectItem value="PENDING">Pending</SelectItem>
            <SelectItem value="ACCEPTED">Accepted</SelectItem>
            <SelectItem value="DECLINED">Declined</SelectItem>
            <SelectItem value="EXPIRED">Expired</SelectItem>
            <SelectItem value="CANCELLED">Cancelled</SelectItem>
          </SelectContent>
        </Select>
        <Select value={sortBy} onValueChange={setSortBy}>
          <SelectTrigger className="w-[180px]">
            <SelectValue placeholder="Sort by" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="created_at_desc">Newest First</SelectItem>
            <SelectItem value="created_at_asc">Oldest First</SelectItem>
            <SelectItem value="quote_number_desc">Quote # (Z-A)</SelectItem>
            <SelectItem value="quote_number_asc">Quote # (A-Z)</SelectItem>
            <SelectItem value="expires_at_desc">Expires (Soonest First)</SelectItem>
            <SelectItem value="expires_at_asc">Expires (Latest First)</SelectItem>
          </SelectContent>
        </Select>
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

  const sendQuoteMutation = useMutation(sendQuote, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listQuotes.service.typeName] })
    },
  })

  const handleSendQuote = async (quoteId: string) => {
    try {
      await sendQuoteMutation.mutateAsync({ id: quoteId })
      toast.success('Quote sent successfully')
    } catch (error) {
      toast.error('Failed to send quote')
    }
  }

  const handleCancelQuote = (quoteId: string) => {
    // TODO: Implement cancel quote functionality
    console.log('Cancelling quote:', quoteId)
  }

  return (
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
            <Button variant="ghost" size="sm" className="h-8 w-8 p-0">
              <ChevronDown className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem asChild>
              <Link to={`${basePath}/quotes/${quote.id}`}>
                <Eye className="w-4 h-4 mr-2" />
                View
              </Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link to={`${basePath}/quotes/${quote.id}/edit`}>
                <Edit className="w-4 h-4 mr-2" />
                Edit
              </Link>
            </DropdownMenuItem>
            {(quote.status === QuoteStatus.DRAFT || quote.status === QuoteStatus.PENDING) && (
              <DropdownMenuItem onClick={() => handleSendQuote(quote.id)}>
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
  )
}

const QuoteStatusBadge: FC<{ status: QuoteStatus }> = ({ status }) => {
  const getBadgeProps = () => {
    switch (status) {
      case QuoteStatus.DRAFT:
        return { variant: 'secondary' as const, children: 'Draft' }
      case QuoteStatus.PENDING:
        return { variant: 'warning' as const, children: 'Pending' }
      case QuoteStatus.ACCEPTED:
        return { variant: 'success' as const, children: 'Accepted' }
      case QuoteStatus.DECLINED:
        return { variant: 'destructive' as const, children: 'Declined' }
      case QuoteStatus.EXPIRED:
        return { variant: 'outline' as const, children: 'Expired' }
      case QuoteStatus.CANCELLED:
        return { variant: 'outline' as const, children: 'Cancelled' }
      default:
        return { variant: 'outline' as const, children: 'Unknown' }
    }
  }

  return <Badge {...getBadgeProps()} />
}
