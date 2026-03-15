import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Label,
  Skeleton,
  Textarea,
} from '@md/ui'

import { RecipientDetails } from '@/rpc/api/quotes/v1/models_pb'

interface SendQuoteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  quoteNumber?: string
  recipients?: RecipientDetails[]
  customMessage: string
  onCustomMessageChange: (msg: string) => void
  onConfirm: () => void
  isPending: boolean
}

export const SendQuoteDialog = ({
  open,
  onOpenChange,
  quoteNumber,
  recipients,
  customMessage,
  onCustomMessageChange,
  onConfirm,
  isPending,
}: SendQuoteDialogProps) => (
  <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent className="max-w-md">
      <DialogHeader>
        <DialogTitle>Send Quote to Customer</DialogTitle>
        <DialogDescription>
          An email will be sent to each recipient with a link to view and sign the quote
          {quoteNumber ? (
            <>
              {' '}
              <span className="font-medium">{quoteNumber}</span>
            </>
          ) : null}
          .
        </DialogDescription>
      </DialogHeader>

      <div className="space-y-4">
        <div>
          <Label>Recipients</Label>
          <div className="mt-2 space-y-2">
            {recipients === undefined ? (
              <>
                <Skeleton height={40} />
                <Skeleton height={40} />
              </>
            ) : recipients.length > 0 ? (
              recipients.map((recipient, index) => (
                <div
                  key={index}
                  className="flex items-center gap-2 p-2 bg-muted/50 rounded-lg text-sm"
                >
                  <div className="flex-1 min-w-0">
                    <div className="font-medium truncate">{recipient.name}</div>
                    <div className="text-muted-foreground truncate">{recipient.email}</div>
                  </div>
                </div>
              ))
            ) : (
              <div className="text-sm text-muted-foreground">No recipients configured</div>
            )}
          </div>
        </div>

        <div>
          <Label htmlFor="send-quote-message">Custom Message (optional)</Label>
          <Textarea
            id="send-quote-message"
            value={customMessage}
            onChange={e => onCustomMessageChange(e.target.value)}
            placeholder="Add a personalized message to include in the email..."
            className="mt-1"
            rows={3}
          />
        </div>
      </div>

      <DialogFooter>
        <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
          Cancel
        </Button>
        <Button onClick={onConfirm} disabled={isPending}>
          {isPending ? 'Sending...' : 'Send Quote'}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
)
