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
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { toast } from 'sonner'

import { minCents, refundableCents } from '@/features/invoices/refunds'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrency, majorToMinorUnit, minorToMajorUnit } from '@/lib/utils/numbers'
import {
  getCreditNote,
  listCreditNoteRefunds,
} from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import {
  getInvoice,
  recordOfflineRefund,
  refundTransaction,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { Transaction } from '@/rpc/api/invoices/v1/models_pb'

export interface RefundCreditNoteOption {
  id: string
  creditNoteNumber: string
  /** What is left to draw on that credit note, in minor units. */
  refundableCents: bigint
}

interface RefundDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The settled payment the money is given back from. */
  transaction: Transaction
  /** The invoice's transactions, to net the refunds already in flight. */
  transactions: Transaction[]
  currency: string
  creditNoteOptions: RefundCreditNoteOption[]
  /** Opened from a credit note: the link is fixed, so no credit-note picker is shown. */
  lockedCreditNoteId?: string
  supportsOnlineRefunds: boolean
  supportsPartialRefunds: boolean
  initialMode?: RefundMode
}

type RefundMode = 'online' | 'offline'

const NO_CREDIT_NOTE = 'none'

const OVERPAYMENT_HELPER =
  'Without a credit note, only an overpayment (a duplicate or excess payment) can be given back, ' +
  'and only the part of it not already used as customer credit.'

const CREDIT_NOTE_HELPER =
  'A refund draws on a finalized refund credit note, which is what reduces the invoice.'

export const RefundDialog: React.FC<RefundDialogProps> = ({
  open,
  onOpenChange,
  transaction,
  transactions,
  currency,
  creditNoteOptions,
  lockedCreditNoteId,
  supportsOnlineRefunds,
  supportsPartialRefunds,
  initialMode,
}) => {
  const queryClient = useQueryClient()

  const onlineAvailable = supportsOnlineRefunds && Boolean(transaction.paymentMethodId)
  const parentRefundable = refundableCents(transaction, transactions)
  const precision = currencyPrecision(currency)

  const step = Math.pow(10, -precision).toFixed(precision)

  const capFor = (option?: RefundCreditNoteOption) =>
    option ? minCents(option.refundableCents, parentRefundable) : parentRefundable

  const lockedOption = creditNoteOptions.find(cn => cn.id === lockedCreditNoteId)
  const maxAmount = minorToMajorUnit(capFor(lockedOption), currency).toFixed(precision)

  const defaultMode: RefundMode = onlineAvailable ? (initialMode ?? 'online') : 'offline'

  const [mode, setMode] = useState<RefundMode>(defaultMode)
  const [amount, setAmount] = useState(maxAmount)
  const [creditNoteId, setCreditNoteId] = useState(lockedCreditNoteId ?? NO_CREDIT_NOTE)
  const [reason, setReason] = useState('')
  const [refundDate, setRefundDate] = useState('')
  const [reference, setReference] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)

  const invalidate = async () => {
    await Promise.all([
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoice,
          input: { id: transaction.invoiceId ?? '' },
          cardinality: 'finite',
        }),
      }),
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: getCreditNote, cardinality: undefined }),
      }),
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listCreditNoteRefunds, cardinality: undefined }),
      }),
    ])
  }

  const onlineMutation = useMutation(refundTransaction, { onSuccess: invalidate })
  const offlineMutation = useMutation(recordOfflineRefund, { onSuccess: invalidate })

  const selectedMaxCents = capFor(creditNoteOptions.find(cn => cn.id === creditNoteId))

  const reset = () => {
    setMode(defaultMode)
    setAmount(maxAmount)
    setCreditNoteId(lockedCreditNoteId ?? NO_CREDIT_NOTE)
    setReason('')
    setRefundDate('')
    setReference('')
  }

  const handleCancel = () => {
    reset()
    onOpenChange(false)
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!amount || parseFloat(amount) <= 0) {
      toast.error('Please enter a valid amount')
      return
    }

    const amountCents = majorToMinorUnit(parseFloat(amount), currency)
    if (amountCents > selectedMaxCents) {
      toast.error(`Amount cannot exceed ${formatCurrency(selectedMaxCents, currency)}`)
      return
    }

    if (mode === 'online' && !supportsPartialRefunds && amountCents !== transaction.amount) {
      toast.error('This provider can only refund the full payment')
      return
    }

    const linkedCreditNoteId = creditNoteId === NO_CREDIT_NOTE ? undefined : creditNoteId

    setIsSubmitting(true)
    try {
      if (mode === 'online') {
        await onlineMutation.mutateAsync({
          transactionId: transaction.id,
          amount,
          creditNoteId: linkedCreditNoteId,
          reason: reason || undefined,
        })
      } else {
        await offlineMutation.mutateAsync({
          transactionId: transaction.id,
          amount,
          creditNoteId: linkedCreditNoteId,
          refundDate: refundDate || undefined,
          reference: reference || undefined,
          reason: reason || undefined,
        })
      }

      toast.success(mode === 'online' ? 'Refund issued' : 'Refund recorded')
      reset()
      onOpenChange(false)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : 'Failed to refund the payment')
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleCancel}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Refund payment</DialogTitle>
          <DialogDescription>
            Gives back part or all of a settled payment. The invoice keeps its amount due and
            payment status — the refund credit note is what reduces it.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4">
          {onlineAvailable && (
            <div className="space-y-2">
              <Label className="text-sm font-medium">How is the money given back?</Label>
              <RadioGroup value={mode} onValueChange={v => setMode(v as RefundMode)}>
                <div className="flex items-start space-x-2">
                  <RadioGroupItem value="online" id="refund-online" className="mt-1" />
                  <Label htmlFor="refund-online" className="text-sm font-normal cursor-pointer">
                    Through the payment provider
                    <span className="block text-xs text-muted-foreground">
                      The provider gives the money back on the original payment method.
                    </span>
                  </Label>
                </div>
                <div className="flex items-start space-x-2">
                  <RadioGroupItem value="offline" id="refund-offline" className="mt-1" />
                  <Label htmlFor="refund-offline" className="text-sm font-normal cursor-pointer">
                    Record a refund made outside Meteroid
                    <span className="block text-xs text-muted-foreground">
                      Wire, cheque or cash already sent back to the customer.
                    </span>
                  </Label>
                </div>
              </RadioGroup>
            </div>
          )}

          <div className="space-y-2">
            <Label htmlFor="refund-amount">
              Amount<span className="text-destructive ml-1">*</span>
            </Label>
            <div className="flex gap-2">
              <Input
                id="refund-amount"
                type="number"
                step={step}
                min={step}
                max={minorToMajorUnit(selectedMaxCents, currency).toFixed(precision)}
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
              Refundable: {formatCurrency(selectedMaxCents, currency)}
            </p>
          </div>

          {lockedOption ? (
            <div className="space-y-2">
              <Label className="text-sm font-medium">Credit note</Label>
              <p className="text-[13px]">{lockedOption.creditNoteNumber}</p>
            </div>
          ) : (
            <div className="space-y-2">
              <Label htmlFor="refund-credit-note">Credit note</Label>
              <Select value={creditNoteId} onValueChange={setCreditNoteId}>
                <SelectTrigger id="refund-credit-note">
                  <SelectValue placeholder="Select a credit note" />
                </SelectTrigger>
                <SelectContent>
                  {creditNoteOptions.map(cn => (
                    <SelectItem key={cn.id} value={cn.id}>
                      {cn.creditNoteNumber} — {formatCurrency(cn.refundableCents, currency)}
                    </SelectItem>
                  ))}
                  <SelectItem value={NO_CREDIT_NOTE}>
                    No credit note — overpayment only
                  </SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {creditNoteId === NO_CREDIT_NOTE ? OVERPAYMENT_HELPER : CREDIT_NOTE_HELPER}
              </p>
            </div>
          )}

          {mode === 'offline' && (
            <>
              <div className="space-y-2">
                <Label htmlFor="refund-date">Refund date</Label>
                <Input
                  id="refund-date"
                  type="datetime-local"
                  value={refundDate}
                  onChange={e => setRefundDate(e.target.value + ':00')}
                  placeholder="Leave empty for current date/time"
                />
                <p className="text-xs text-muted-foreground">
                  Optional. Defaults to current date/time if not specified.
                </p>
              </div>

              <div className="space-y-2">
                <Label htmlFor="refund-reference">Reference / Note</Label>
                <Input
                  id="refund-reference"
                  type="text"
                  value={reference}
                  onChange={e => setReference(e.target.value)}
                  placeholder="e.g., Wire ref. 12345"
                />
              </div>
            </>
          )}

          <div className="space-y-2">
            <Label htmlFor="refund-reason">Reason</Label>
            <Input
              id="refund-reason"
              type="text"
              value={reason}
              onChange={e => setReason(e.target.value)}
              placeholder="e.g., Customer requested a refund"
            />
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={handleCancel} disabled={isSubmitting}>
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting || selectedMaxCents <= 0n}>
              {isSubmitting ? 'Refunding...' : 'Refund'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
