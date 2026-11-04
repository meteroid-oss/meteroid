import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { toast } from 'sonner'

import {
  DISPUTE_REVERSAL_KINDS,
  REVERSAL_KINDS,
  REVERSAL_KIND_LABELS,
} from '@/features/invoices/refunds'
import { formatCurrency } from '@/lib/utils/numbers'
import {
  getInvoice,
  reverseManualPayment,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { Transaction, Transaction_ReversalKind } from '@/rpc/api/invoices/v1/models_pb'

interface ReverseManualPaymentDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  transaction: Transaction
  currency: string
}

export const ReverseManualPaymentDialog: React.FC<ReverseManualPaymentDialogProps> = ({
  open,
  onOpenChange,
  transaction,
  currency,
}) => {
  const queryClient = useQueryClient()

  const [kind, setKind] = useState<Transaction_ReversalKind>(
    Transaction_ReversalKind.REVERSAL_RETURNED
  )
  const [reason, setReason] = useState('')
  const [reversedAt, setReversedAt] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)

  const reverseMutation = useMutation(reverseManualPayment, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoice,
          input: { id: transaction.invoiceId ?? '' },
          cardinality: 'finite',
        }),
      })
    },
  })

  const reset = () => {
    setKind(Transaction_ReversalKind.REVERSAL_RETURNED)
    setReason('')
    setReversedAt('')
  }

  const handleCancel = () => {
    reset()
    onOpenChange(false)
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    setIsSubmitting(true)
    try {
      await reverseMutation.mutateAsync({
        transactionId: transaction.id,
        kind,
        reason: reason || undefined,
        reversedAt: reversedAt || undefined,
      })

      toast.success('Payment reversed')
      reset()
      onOpenChange(false)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : 'Failed to reverse the payment')
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleCancel}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Reverse payment</DialogTitle>
          <DialogDescription>
            {DISPUTE_REVERSAL_KINDS.includes(kind)
              ? `The ${formatCurrency(transaction.amount, currency)} is lost to the payer's claim. The invoice stays paid and the loss is recorded.`
              : `The ${formatCurrency(transaction.amount, currency)} never really landed. The payment is clawed back and the invoice reopens as unpaid.`}
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="reversal-kind">
              Cause<span className="text-destructive ml-1">*</span>
            </Label>
            <Select
              value={String(kind)}
              onValueChange={v => setKind(Number(v) as Transaction_ReversalKind)}
            >
              <SelectTrigger id="reversal-kind">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {REVERSAL_KINDS.map(k => (
                  <SelectItem key={k} value={String(k)}>
                    {REVERSAL_KIND_LABELS[k]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <Label htmlFor="reversal-date">Reversal date</Label>
            <Input
              id="reversal-date"
              type="datetime-local"
              value={reversedAt}
              onChange={e => setReversedAt(e.target.value + ':00')}
              placeholder="Leave empty for current date/time"
            />
            <p className="text-xs text-muted-foreground">
              Optional. Defaults to current date/time if not specified.
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="reversal-reason">Reason</Label>
            <Input
              id="reversal-reason"
              type="text"
              value={reason}
              onChange={e => setReason(e.target.value)}
              placeholder="e.g., Bank returned the transfer"
            />
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={handleCancel} disabled={isSubmitting}>
              Cancel
            </Button>
            <Button type="submit" variant="destructive" disabled={isSubmitting}>
              {isSubmitting ? 'Reversing...' : 'Reverse payment'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
