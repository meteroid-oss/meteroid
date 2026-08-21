import { Button, Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@md/ui'
import { Check, Copy, ExternalLink } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

interface Props {
  /** The payment link to share, or null when the dialog is closed. */
  url: string | null
  onClose: () => void
}

/**
 * A tiny modal that surfaces a freshly generated invoice payment link so it can be
 * copied and shared, rather than opening it in a new tab.
 */
export const SharePaymentLinkDialog = ({ url, onClose }: Props) => {
  const [copied, setCopied] = useState(false)

  const copy = async () => {
    if (!url) return
    await navigator.clipboard.writeText(url)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
    toast.success('Payment link copied to clipboard')
  }

  return (
    <Dialog
      open={!!url}
      onOpenChange={open => {
        if (!open) {
          setCopied(false)
          onClose()
        }
      }}
    >
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>Share payment link</DialogTitle>
          <DialogDescription>
            Send this secure link to your customer so they can pay this invoice online.
          </DialogDescription>
        </DialogHeader>

        <div className="flex items-center gap-2">
          <input
            readOnly
            value={url ?? ''}
            onFocus={e => e.currentTarget.select()}
            className="flex-1 min-w-0 rounded-md border border-border bg-muted px-3 py-2 text-xs font-mono text-foreground outline-none focus:ring-1 focus:ring-ring"
          />
          <Button variant="secondary" size="icon" onClick={copy} aria-label="Copy payment link">
            {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
          </Button>
        </div>

        <div className="flex justify-end">
          {url && (
            <Button variant="ghost" size="sm" asChild>
              <a href={url} target="_blank" rel="noopener noreferrer">
                <ExternalLink className="mr-2 h-4 w-4" />
                Open in new tab
              </a>
            </Button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
