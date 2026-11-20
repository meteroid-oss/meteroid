import {
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Label,
  Skeleton,
  Textarea,
} from '@md/ui'

import { useTenant } from '@/hooks/useTenant'
import { EmailDeliveryMode } from '@/rpc/api/tenants/v1/models_pb'

interface SendQuoteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  quoteNumber?: string
  /** Who is emailed: every recipient, or the one person a reminder is for. */
  recipients?: { name: string; email: string }[]
  /** The quote is already out: the email reminds of the current version. */
  reminder?: boolean
  customMessage: string
  onCustomMessageChange: (msg: string) => void
  onConfirm: () => void
  isPending: boolean
  emailEnabled?: boolean
}

export const SendQuoteDialog = ({
  open,
  onOpenChange,
  quoteNumber,
  recipients,
  reminder = false,
  customMessage,
  onCustomMessageChange,
  onConfirm,
  isPending,
  emailEnabled,
}: SendQuoteDialogProps) => {
  const { tenant } = useTenant()
  const deliveryMode = tenant?.emailDeliveryMode

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md" aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle>
            {reminder ? 'Send a reminder' : 'Send to customer'}
            {quoteNumber && (
              <span className="font-normal text-muted-foreground"> · {quoteNumber}</span>
            )}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {emailEnabled === false && (
            <div className="rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
              Quote emails are off (Settings › Customer emails).
            </div>
          )}
          {emailEnabled !== false && deliveryMode === EmailDeliveryMode.OFF && (
            <div className="rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
              Customer emails are off: nothing will be delivered.
            </div>
          )}
          {emailEnabled !== false && deliveryMode === EmailDeliveryMode.TEST && (
            <div className="rounded-lg border border-border bg-muted/50 p-3 text-sm">
              Test mode: only your team and test recipients receive it.
            </div>
          )}
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
                <div className="text-sm text-muted-foreground">No recipients</div>
              )}
            </div>
          </div>

          <div>
            <Label htmlFor="send-quote-message">Message (optional)</Label>
            <Textarea
              id="send-quote-message"
              value={customMessage}
              onChange={e => onCustomMessageChange(e.target.value)}
              className="mt-1"
              rows={3}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            Cancel
          </Button>
          <Button onClick={onConfirm} disabled={isPending || emailEnabled === false}>
            {isPending ? 'Sending...' : reminder ? 'Send reminder' : 'Send Quote'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
