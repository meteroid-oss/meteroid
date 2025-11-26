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
import { useState } from 'react'
import { toast } from 'sonner'

import {
  addManualPaymentTransaction,
  getInvoice,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'

interface AddManualPaymentDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  invoiceId: string
  currency: string
  maxAmount: string
}

export const AddManualPaymentDialog: React.FC<AddManualPaymentDialogProps> = ({
  open,
  onOpenChange,
  invoiceId,
  currency,
  maxAmount,
}) => {
  const queryClient = useQueryClient()
  const [amount, setAmount] = useState('')
  const [paymentDate, setPaymentDate] = useState('')
  const [reference, setReference] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)

  const addPaymentMutation = useMutation(addManualPaymentTransaction, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getInvoice, { id: invoiceId }),
      })
    },
  })

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!amount || parseFloat(amount) <= 0) {
      toast.error('Please enter a valid amount')
      return
    }

    const amountNum = parseFloat(amount)
    const maxAmountNum = parseFloat(maxAmount)

    if (amountNum > maxAmountNum) {
      toast.error(`Amount cannot exceed ${maxAmount} ${currency}`)
      return
    }

    setIsSubmitting(true)

    try {
      await addPaymentMutation.mutateAsync({
        invoiceId,
        amount,
        paymentDate: paymentDate ? paymentDate : undefined,
        reference: reference || undefined,
      })

      toast.success('Payment transaction added successfully')

      // Reset form and close dialog
      setAmount('')
      setPaymentDate('')
      setReference('')
      onOpenChange(false)
    } catch (error) {
      console.error('Error adding manual payment:', error)
      toast.error(error instanceof Error ? error.message : 'Failed to add payment')
    } finally {
      setIsSubmitting(false)
    }
  }

  const handleCancel = () => {
    setAmount('')
    setPaymentDate('')
    setReference('')
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Manual Payment</DialogTitle>
          <DialogDescription>
            Record a payment received outside the system (e.g., bank transfer, cash, check).
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="amount">
              Amount<span className="text-destructive ml-1">*</span>
            </Label>
            <div className="flex gap-2">
              <Input
                id="amount"
                type="number"
                step="0.01"
                min="0.01"
                max={maxAmount}
                placeholder="0.00"
                value={amount}
                onChange={e => setAmount(e.target.value)}
                required
                className="flex-1"
              />
              <div className="flex items-center px-3 bg-muted rounded-md min-w-[60px] justify-center">
                <span className="text-sm font-medium">{currency}</span>
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              Maximum: {maxAmount} {currency}
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="paymentDate">Payment Date</Label>
            <Input
              id="paymentDate"
              type="datetime-local"
              value={paymentDate}
              onChange={e => setPaymentDate(e.target.value + ':00')}
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
              {isSubmitting ? 'Adding Payment...' : 'Add Payment'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
