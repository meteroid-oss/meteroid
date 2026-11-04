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

import { useTenant } from '@/hooks/useTenant'
import { RecipientDetails } from '@/rpc/api/quotes/v1/models_pb'
import { EmailDeliveryMode } from '@/rpc/api/tenants/v1/models_pb'

interface SendQuoteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  quoteNumber?: string
  recipients?: RecipientDetails[]
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
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Send Quote to Customer</DialogTitle>
          <DialogDescription>
            Publishes the quote and emails each recipient a link to view and sign it
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
          {emailEnabled === false && (
            <div className="rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
              The Quote ready email is turned off in Settings › Customer emails. Turn it on to send
              quotes, or publish the quote and share its link instead.
            </div>
          )}
          {emailEnabled !== false && deliveryMode === EmailDeliveryMode.OFF && (
            <div className="rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
              Customer emails are off for this workspace: no email will be delivered. The quote is
              still published and you can share its link.
            </div>
          )}
          {emailEnabled !== false && deliveryMode === EmailDeliveryMode.TEST && (
            <div className="rounded-lg border border-border bg-muted/50 p-3 text-sm">
              Customer emails are in Test mode: only your team and the test recipients will receive
              this email.
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
          <Button onClick={onConfirm} disabled={isPending || emailEnabled === false}>
            {isPending ? 'Sending...' : 'Send Quote'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
