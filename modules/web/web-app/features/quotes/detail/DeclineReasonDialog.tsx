import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Label,
  Textarea,
} from '@md/ui'
import { useState } from 'react'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  confirmLabel: string
  isPending: boolean
  onConfirm: (reason: string | undefined) => void
}

export const DeclineReasonDialog = ({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  isPending,
  onConfirm,
}: Props) => {
  const [reason, setReason] = useState('')

  return (
    <Dialog
      open={open}
      onOpenChange={next => {
        if (!next) setReason('')
        onOpenChange(next)
      }}
    >
      <DialogContent
        className="max-w-md"
        {...(description ? {} : { 'aria-describedby': undefined })}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>
        <div>
          <Label htmlFor="decline-reason">Reason (optional)</Label>
          <Textarea
            id="decline-reason"
            value={reason}
            onChange={e => setReason(e.target.value)}
            className="mt-1"
            rows={3}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={() => onConfirm(reason.trim() || undefined)}
            disabled={isPending}
          >
            {isPending ? 'Declining...' : confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
