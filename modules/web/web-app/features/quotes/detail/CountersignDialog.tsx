import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
} from '@md/ui'
import { useState } from 'react'
import { toast } from 'sonner'

import { DEFAULT_CONSENT_TEXT, rpcErrorMessage } from '@/features/quotes/detail/signers'
import { ConsentCheckbox } from '@/features/quotes/portal/ConsentCheckbox'
import { DetailedQuote, QuoteCountersigner } from '@/rpc/api/quotes/v1/models_pb'
import { countersignQuote } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  quote: DetailedQuote
  countersigner: QuoteCountersigner
  onSigned: () => void
}

export const CountersignDialog = ({
  open,
  onOpenChange,
  quote,
  countersigner,
  onSigned,
}: Props) => {
  const consentText = quote.quote?.consentText || DEFAULT_CONSENT_TEXT
  const [consented, setConsented] = useState(false)
  const [title, setTitle] = useState(countersigner.title ?? '')
  const mutation = useMutation(countersignQuote)

  const submit = async () => {
    if (!quote.quote) return
    try {
      await mutation.mutateAsync({
        quoteId: quote.quote.id,
        documentHash: quote.quote.documentHash ?? '',
        consentText,
        title: title.trim() || undefined,
      })
      toast.success('Quote countersigned')
      onOpenChange(false)
      onSigned()
    } catch (error) {
      toast.error(rpcErrorMessage(error, 'Failed to countersign the quote'))
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={next => {
        if (!next) setConsented(false)
        onOpenChange(next)
      }}
    >
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Countersign quote {quote.quote?.quoteNumber}</DialogTitle>
          <DialogDescription>
            Signing as {countersigner.name} ({countersigner.email})
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div>
            <Label htmlFor="countersign-title">Title (optional)</Label>
            <Input
              id="countersign-title"
              value={title}
              onChange={e => setTitle(e.target.value)}
              className="mt-1"
            />
          </div>
          <ConsentCheckbox text={consentText} checked={consented} onCheckedChange={setConsented} />
        </div>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={mutation.isPending}
          >
            Cancel
          </Button>
          <Button onClick={submit} disabled={!consented || mutation.isPending}>
            {mutation.isPending ? 'Signing...' : 'Countersign'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
