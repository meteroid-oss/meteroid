import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Flex,
  Input,
  Label,
  Separator,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ChevronDown, Copy, Download, Edit, ExternalLink, FileText, Send } from 'lucide-react'
import { Fragment, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { QuoteView } from '@/features/quotes/QuoteView'
import { formatSubscriptionFee } from '@/features/subscriptions/utils/fees'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import {
  DetailedQuote,
  QuoteComponent,
  QuoteSignature,
  QuoteStatus,
  RecipientDetails,
} from '@/rpc/api/quotes/v1/models_pb'
import {
  generateQuotePortalToken,
  getQuote,
  listQuotes,
  publishQuote,
} from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { parseAndFormatDate } from '@/utils/date'
import { useTypedParams } from '@/utils/params'

export const Quote = () => {
  const { quoteId } = useTypedParams<{ quoteId: string }>()

  const quoteQuery = useQuery(getQuote, { id: quoteId ?? '' }, { enabled: Boolean(quoteId) })

  const data = quoteQuery.data?.quote
  const isLoading = quoteQuery.isLoading

  return (
    <Fragment>
      <Flex direction="column" className="h-full">
        {isLoading || !data ? (
          <>
            <Skeleton height={16} width={50} />
            <Skeleton height={44} />
          </>
        ) : (
          <QuoteDetailView quote={data} />
        )}
      </Flex>
    </Fragment>
  )
}

interface Props {
  quote: DetailedQuote
}

export const QuoteDetailView: React.FC<Props> = ({ quote }) => {
  const basePath = useBasePath()

  const queryClient = useQueryClient()

  const publishQuoteMutation = useMutation(publishQuote, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listQuotes.service.typeName] })
      await queryClient.invalidateQueries({ queryKey: [getQuote.service.typeName] })
    },
  })

  const canEdit = quote.quote?.status === QuoteStatus.DRAFT
  const canPublish = quote.quote?.status === QuoteStatus.DRAFT
  const canSend =
    quote.quote?.status === QuoteStatus.DRAFT || quote.quote?.status === QuoteStatus.PENDING
  const canConvert = quote.quote?.status === QuoteStatus.ACCEPTED

  const [showTokenDialog, setShowTokenDialog] = useState(false)
  const [recipientEmail, setRecipientEmail] = useState('')
  const [portalUrl, setPortalUrl] = useState('')

  const generateTokenMutation = useMutation(generateQuotePortalToken)

  const handlePublishQuote = async () => {
    if (!quote.quote?.id) return

    try {
      await publishQuoteMutation.mutateAsync({
        id: quote.quote.id,
      })
      toast.success('Quote published successfully')
      // Optionally refetch the quote to update the status
      window.location.reload() // Simple reload, or you could use the query's refetch
    } catch (error) {
      toast.error('Failed to publish quote')
    }
  }

  const handleGenerateToken = async () => {
    if (!recipientEmail || !quote.quote?.id) return

    try {
      const response = await generateTokenMutation.mutateAsync({
        quoteId: quote.quote.id,
        recipientEmail: recipientEmail,
      })

      setPortalUrl(`${window.location.origin}/quote?token=${response.token}`)
    } catch (error) {
      toast.error('Failed to generate portal token')
    }
  }

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text)
  }

  const openTokenDialog = () => {
    setRecipientEmail(quote.customer?.billingEmail || '')
    setPortalUrl('')
    setShowTokenDialog(true)
  }

  const openTokenDialogForRecipient = async (email: string) => {
    if (!quote.quote?.id) return

    try {
      const response = await generateTokenMutation.mutateAsync({
        quoteId: quote.quote.id,
        recipientEmail: email,
      })

      const url = `${window.location.origin}/quote?token=${response.token}`
      copyToClipboard(url)
      toast.success('Portal URL copied to clipboard!', { id: 'copy' })
    } catch (error) {
      toast.error('Failed to generate portal token')
    }
  }

  return (
    <Flex className="h-full">
      {/* Left Panel - Quote Details */}
      <Flex direction="column" className="w-1/3 border-r border-border">
        {/* Fixed Header */}
        <Flex direction="column" className="gap-2 p-6 border-b border-border">
          <div className="flex justify-between items-center">
            <div className="flex items-center gap-3">
              <QuoteStatusBadge status={quote.quote?.status || QuoteStatus.DRAFT} />
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" size="sm" hasIcon>
                  Actions
                  <ChevronDown className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {canEdit && (
                  <DropdownMenuItem asChild>
                    <Link to={`${basePath}/quotes/${quote.quote?.id}/edit`}>
                      <Edit size="16" className="mr-2" />
                      Edit Quote
                    </Link>
                  </DropdownMenuItem>
                )}
                {canPublish && (
                  <DropdownMenuItem onClick={handlePublishQuote}>
                    <FileText size="16" className="mr-2" />
                    Publish Quote
                  </DropdownMenuItem>
                )}
                {canSend && (
                  <DropdownMenuItem>
                    <Send size="16" className="mr-2" />
                    Send to Customer
                  </DropdownMenuItem>
                )}

                {canConvert && (
                  <DropdownMenuItem asChild>
                    <Link to={`${basePath}/quotes/${quote.quote?.id}/convert`}>
                      <FileText size="16" className="mr-2" />
                      Convert to Subscription
                    </Link>
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem disabled={true}>
                  <Download size="16" className="mr-2" />
                  Download PDF
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onClick={openTokenDialog}>
                  <ExternalLink size="16" className="mr-2" />
                  Generate Portal Link
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          <div className="text-xl font-bold">Quote {quote.quote?.quoteNumber}</div>
        </Flex>

        {/* Scrollable Content */}
        <div className="flex-1 overflow-auto">
          <Flex direction="column" className="gap-2 p-6">
            <FlexDetails title="Quote number" value={quote.quote?.quoteNumber} />
            <FlexDetails
              title="Created"
              value={quote.quote?.createdAt ? parseAndFormatDate(quote.quote.createdAt) : '—'}
            />
            <FlexDetails
              title="Expires"
              value={quote.quote?.expiresAt ? parseAndFormatDate(quote.quote.expiresAt) : '—'}
            />
            <FlexDetails title="Currency" value={quote.quote?.currency} />
            <FlexDetails title="Plan Version" value={quote.quote?.planVersionId} />
          </Flex>

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Recipients</div>
            <QuoteRecipients
              recipients={quote.quote?.recipients || []}
              signatures={quote.signatures || []}
              onGenerateToken={email => openTokenDialogForRecipient(email)}
            />
          </Flex>

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Customer</div>
            <FlexDetails
              title="Customer"
              value={quote.customer?.name || 'Customer'}
              link={`${basePath}/customers/${quote.quote?.customerId}`}
            />
            <FlexDetails title="Customer ID" value={quote.quote?.customerId} />
            <FlexDetails title="Email" value={quote.customer?.billingEmail} />
          </Flex>

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Subscription Components</div>
            <QuoteComponents components={quote.components || []} quote={quote} />
          </Flex>

          {(quote.quote?.overview ||
            quote.quote?.termsAndServices ||
            quote.quote?.internalNotes) && (
            <>
              <Separator className="-my-3" />
              <Flex direction="column" className="gap-2 p-6">
                <div className="text-[15px] font-medium">Additional Information</div>

                {quote.quote?.internalNotes && (
                  <div className="mt-2">
                    <div className="text-[13px] font-medium mb-1 text-orange-600">
                      Internal Notes
                    </div>
                    <div className="text-[13px] text-muted-foreground whitespace-pre-line">
                      {quote.quote.internalNotes}
                    </div>
                  </div>
                )}
              </Flex>
            </>
          )}

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Timeline</div>
            <div className="space-y-2">
              {quote.activities?.map((activity, index) => (
                <div key={activity.id || index} className="flex items-start gap-2">
                  <div className="w-1.5 h-1.5 rounded-full bg-muted-foreground mt-1.5 flex-shrink-0"></div>
                  <div>
                    <div className="text-[13px] font-medium">{activity.description}</div>
                    <div className="text-[11px] text-muted-foreground">
                      {parseAndFormatDate(activity.createdAt)}
                    </div>
                  </div>
                </div>
              )) || <div className="text-[13px] text-muted-foreground">No activity recorded</div>}
            </div>
          </Flex>
        </div>
      </Flex>

      {/* Right Panel - Quote Preview */}
      <div className="w-2/3 flex flex-col">
        <div className="flex-1 overflow-auto p-6">
          <QuoteView quote={quote} mode="detailed" />
        </div>
      </div>

      {/* Generate Portal Token Dialog */}
      <Dialog open={showTokenDialog} onOpenChange={setShowTokenDialog}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Generate Portal Link</DialogTitle>
            <DialogDescription>
              Create a secure link that allows the recipient to view and sign the quote.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            <div>
              <Label htmlFor="recipient-email">Recipient Email</Label>
              <Input
                id="recipient-email"
                type="email"
                value={recipientEmail}
                onChange={e => setRecipientEmail(e.target.value)}
                placeholder="Enter recipient email"
                className="mt-1"
              />
            </div>

            {portalUrl && (
              <div>
                <Label>Portal URL</Label>
                <div className="flex gap-2 mt-1">
                  <Input value={portalUrl} readOnly />
                  <Button size="sm" variant="outline" onClick={() => copyToClipboard(portalUrl)}>
                    <Copy className="w-4 h-4" />
                  </Button>
                </div>
                <p className="text-sm text-muted-foreground mt-1">
                  Share this link with the recipient to allow them to view and sign the quote.
                </p>
              </div>
            )}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowTokenDialog(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleGenerateToken}
              disabled={!recipientEmail || generateTokenMutation.isPending}
            >
              {generateTokenMutation.isPending ? 'Generating...' : 'Generate Link'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Flex>
  )
}

const QuoteStatusBadge = ({ status }: { status: QuoteStatus }) => {
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

const QuoteComponents: React.FC<{ components: QuoteComponent[]; quote: DetailedQuote }> = ({
  components,
  quote,
}) => {
  return (
    <div className="space-y-2">
      {components.length > 0 ? (
        components.map(component => (
          <QuoteComponentCard key={component.id} component={component} quote={quote} />
        ))
      ) : (
        <div className="text-[13px] text-muted-foreground py-2">
          No subscription components configured
        </div>
      )}
    </div>
  )
}

const QuoteComponentCard: React.FC<{
  component: QuoteComponent
  quote: DetailedQuote
}> = ({ component, quote }) => {
  if (!quote.quote?.currency) return null

  const formatted = formatSubscriptionFee(component.fee, quote.quote.currency)

  console.log('formatted', formatted, component)

  return (
    <div className="py-2">
      <div className="flex justify-between items-start">
        <div className="flex-1">
          <div className="text-[13px] font-medium">{component.name}</div>
          <div className="text-[11px] text-muted-foreground mt-1">
            {component.period}
            {component.isOverride && ' • Custom pricing'}
          </div>
        </div>
        <div className="text-right">
          <div className="text-[13px] font-medium">
            <div className="flex justify-between">
              <span>{formatted.details}</span>
            </div>
            <div className="flex justify-between">
              <span className="font-medium text-foreground">{formatted.amount}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

const FlexDetails = ({
  title,
  value,
  externalLink,
  link,
}: {
  title: string | React.ReactNode
  value?: string | React.ReactNode
  externalLink?: string
  link?: string
}) => (
  <Flex align="start" justify="between">
    <div className="text-[13px] text-muted-foreground">{title}</div>
    {externalLink ? (
      <a href={externalLink} target="_blank" rel="noopener noreferrer">
        <div className="text-[13px] text-brand hover:underline">{value ?? 'N/A'}</div>
      </a>
    ) : link ? (
      <Link to={link}>
        <div className="text-[13px] text-brand hover:underline">{value ?? 'N/A'}</div>
      </Link>
    ) : (
      <div className="text-[13px]">{value ?? 'N/A'}</div>
    )}
  </Flex>
)

interface QuoteRecipientsProps {
  recipients: RecipientDetails[]
  signatures: QuoteSignature[]
  onGenerateToken: (email: string) => void
}

const QuoteRecipients: React.FC<QuoteRecipientsProps> = ({
  recipients,
  signatures,
  onGenerateToken,
}) => {
  if (recipients.length === 0) {
    return <div className="text-[13px] text-muted-foreground">No recipients configured</div>
  }

  return (
    <div className="space-y-2">
      {recipients.map((recipient, index) => {
        const hasSigned = signatures.some(s => s.signedByEmail === recipient.email)

        return (
          <div key={index} className="flex items-center justify-between p-2 bg-muted/50 rounded-lg">
            <div className="flex-1 min-w-0">
              <div className="text-[13px] font-medium truncate">{recipient.name}</div>
              <div className="text-[11px] text-muted-foreground truncate">{recipient.email}</div>
            </div>
            <div className="flex items-center gap-2">
              {hasSigned ? (
                <Badge variant="success" className="text-[10px] px-1.5 py-0.5">
                  Signed
                </Badge>
              ) : (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => onGenerateToken(recipient.email)}
                  className="h-6 px-2 text-[10px]"
                >
                  <ExternalLink className="w-3 h-3 mr-1" />
                  Portal Link
                </Button>
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}
