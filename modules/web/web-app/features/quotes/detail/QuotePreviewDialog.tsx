import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
} from '@md/ui'
import { AlertCircle } from 'lucide-react'
import { useEffect, useState } from 'react'

import QuotePortalView from '@/features/quotes/QuotePortalView'
import { rpcErrorMessage } from '@/features/quotes/detail/signers'
import { useQuery } from '@/lib/connectrpc'
import { DetailedQuote } from '@/rpc/api/quotes/v1/models_pb'
import { previewQuotePortal } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  quote: DetailedQuote
}

export const QuotePreviewDialog = ({ open, onOpenChange, quote }: Props) => {
  const recipients = quote.quote?.recipients ?? []
  const quoteId = quote.quote?.id
  const [recipientEmail, setRecipientEmail] = useState(recipients[0]?.email)

  useEffect(() => {
    if (open) setRecipientEmail(recipients[0]?.email)
    // Back to the first recipient each time the preview opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const preview = useQuery(
    previewQuotePortal,
    { quoteId: quoteId ?? '', recipientEmail },
    { enabled: open && Boolean(quoteId) }
  )
  const details = preview.data?.quote

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="max-w-6xl w-[95vw] max-h-[92vh] overflow-y-auto p-0"
        aria-describedby={undefined}
      >
        <DialogHeader className="flex flex-row flex-wrap items-center gap-3 px-6 pt-6 pr-12">
          <DialogTitle>Preview as customer</DialogTitle>
          {recipients.length > 1 && (
            <Select value={recipientEmail} onValueChange={setRecipientEmail}>
              <SelectTrigger className="h-8 w-auto min-w-[200px]" aria-label="Recipient">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {recipients.map(recipient => (
                  <SelectItem key={recipient.email} value={recipient.email}>
                    {recipient.name ? `${recipient.name} (${recipient.email})` : recipient.email}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </DialogHeader>
        {preview.error ? (
          <div className="flex items-center gap-2 px-6 pb-6 text-sm text-destructive">
            <AlertCircle className="h-4 w-4" />
            {rpcErrorMessage(preview.error, 'The preview could not be loaded')}
          </div>
        ) : details ? (
          <QuotePortalView key={recipientEmail} quoteData={details} preview />
        ) : (
          <div className="space-y-3 px-6 pb-6">
            <Skeleton height={48} />
            <Skeleton height={320} />
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
