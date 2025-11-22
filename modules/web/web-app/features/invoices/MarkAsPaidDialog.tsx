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
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { AlertCircle } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import {
  getInvoice,
  markInvoiceAsPaid,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'

interface MarkAsPaidDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  invoiceId: string
  invoiceNumber: string
  currency: string
  totalAmount: string
}

export const MarkAsPaidDialog: React.FC<MarkAsPaidDialogProps> = ({
  open,
  onOpenChange,
  invoiceId,
  invoiceNumber,
  currency,
  totalAmount,
}) => {
  const queryClient = useQueryClient()
  const [paymentDate, setPaymentDate] = useState('')
  const [reference, setReference] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)

  const markAsPaidMutation = useMutation(markInvoiceAsPaid, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getInvoice, { id: invoiceId }),
      })
    },
  })

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    setIsSubmitting(true)

    try {
      await markAsPaidMutation.mutateAsync({
        invoiceId,
        totalAmount,
        paymentDate: paymentDate || undefined,
        reference: reference || undefined,
      })

      toast.success(`Invoice ${invoiceNumber} marked as paid`)

      // Reset form and close dialog
      setPaymentDate('')
      setReference('')
      onOpenChange(false)
    } catch (error) {
      console.error('Error marking invoice as paid:', error)
      toast.error(error instanceof Error ? error.message : 'Failed to mark invoice as paid')
    } finally {
      setIsSubmitting(false)
    }
  }

  const handleCancel = () => {
    setPaymentDate('')
    setReference('')
    onOpenChange(false)
  }

  const displayAmount = (Number(totalAmount) / 100).toFixed(2)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Mark Invoice as Paid</DialogTitle>
          <DialogDescription>
            This will mark invoice {invoiceNumber} as fully paid by recording a manual payment
            transaction for the entire amount due.
          </DialogDescription>
        </DialogHeader>

        <div className="bg-muted/50 rounded-lg p-4 flex items-start gap-3">
          <AlertCircle className="w-5 h-5 text-muted-foreground mt-0.5 flex-shrink-0" />
          <div className="text-sm text-muted-foreground">
            <div className="font-medium mb-1">Amount to be paid:</div>
            <div className="text-lg font-semibold text-foreground">
              {displayAmount} {currency}
            </div>
            <div className="mt-2">
              This amount must match the invoice&apos;s amount due exactly. A payment transaction
              will be created and the invoice status will be updated to Paid.
            </div>
          </div>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="paymentDate">Payment Date</Label>
            <Input
              id="paymentDate"
              type="datetime-local"
              value={paymentDate}
              onChange={e => setPaymentDate(e.target.value)}
              placeholder="Leave empty for current date/time"
            />
            <p className="text-xs text-muted-foreground">
              Optional. Defaults to current date/time if not specified.
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="reference">Reference / Note</Label>
            <Input
              id="reference"
              type="text"
              value={reference}
              onChange={e => setReference(e.target.value)}
              placeholder="e.g., Check #12345 or Bank Transfer"
            />
            <p className="text-xs text-muted-foreground">
              Optional. Add a reference or note for this payment.
            </p>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={handleCancel} disabled={isSubmitting}>
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {isSubmitting ? 'Marking as Paid...' : 'Mark as Paid'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
