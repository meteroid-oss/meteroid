import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Checkbox,
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
  Textarea,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { useBasePath } from '@/hooks/useBasePath'
import { formatCurrency } from '@/lib/utils/numbers'
import { createCreditNote } from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import { CreditType } from '@/rpc/api/creditnotes/v1/models_pb'
import { getInvoice } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { DetailedInvoice, LineItem } from '@/rpc/api/invoices/v1/models_pb'

interface CreateCreditNoteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  invoice: DetailedInvoice
}

interface LineItemSelection {
  lineItemLocalId: string
  selected: boolean
  amount: string // Store as string for input handling
  maxAmount: number
  name: string
}

export const CreateCreditNoteDialog: React.FC<CreateCreditNoteDialogProps> = ({
  open,
  onOpenChange,
  invoice,
}) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  // Initialize line items from invoice
  const initialLineItems: LineItemSelection[] = invoice.lineItems.map((item: LineItem) => ({
    lineItemLocalId: item.id,
    selected: false,
    amount: '',
    maxAmount: Math.abs(Number(item.subtotal)),
    name: item.name,
  }))

  console.log('Initial Line Itemséé:', invoice.lineItems)
  console.log('Initial Line Items:', initialLineItems)

  const [lineItems, setLineItems] = useState<LineItemSelection[]>(initialLineItems)
  const [creditType, setCreditType] = useState<CreditType>(CreditType.CREDIT_TO_BALANCE)
  const [reason, setReason] = useState('')
  const [memo, setMemo] = useState('')

  const createCreditNoteMutation = useMutation(createCreditNote, {
    onSuccess: async data => {
      toast.success('Credit note created successfully')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getInvoice, { id: invoice.id }),
      })
      onOpenChange(false)
      // Navigate to the new credit note
      if (data.creditNote?.id) {
        navigate(`${basePath}/credit-notes/${data.creditNote.id}`)
      }
    },
    onError: error => {
      toast.error(`Failed to create credit note: ${error.message}`)
    },
  })

  const handleToggleLineItem = (localId: string) => {
    setLineItems(prev =>
      prev.map(item =>
        item.lineItemLocalId === localId ? { ...item, selected: !item.selected } : item
      )
    )
  }

  const handleAmountChange = (localId: string, value: string) => {
    setLineItems(prev =>
      prev.map(item => (item.lineItemLocalId === localId ? { ...item, amount: value } : item))
    )
  }

  const handleSelectAll = () => {
    const allSelected = lineItems.every(item => item.selected)
    setLineItems(prev => prev.map(item => ({ ...item, selected: !allSelected })))
  }

  const handleSubmit = () => {
    const selectedItems = lineItems.filter(item => item.selected)

    if (selectedItems.length === 0) {
      toast.error('Please select at least one line item to credit')
      return
    }

    const creditLineItems = selectedItems.map(item => {
      const amount = item.amount ? Math.round(parseFloat(item.amount) * 100) : undefined
      return {
        lineItemLocalId: item.lineItemLocalId,
        amount: amount ? BigInt(amount) : undefined,
      }
    })

    createCreditNoteMutation.mutate({
      creditNote: {
        invoiceId: invoice.id,
        lineItems: creditLineItems,
        reason: reason || undefined,
        memo: memo || undefined,
        creditType,
      },
    })
  }

  const selectedCount = lineItems.filter(item => item.selected).length
  const totalCreditAmount = lineItems
    .filter(item => item.selected)
    .reduce((sum, item) => {
      const amount = item.amount ? parseFloat(item.amount) * 100 : item.maxAmount
      return sum + amount
    }, 0)

  const handleClose = () => {
    // Reset state when closing
    setLineItems(initialLineItems)
    setCreditType(CreditType.CREDIT_TO_BALANCE)
    setReason('')
    setMemo('')
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Create Credit Note</DialogTitle>
          <DialogDescription>
            Create a credit note for invoice {invoice.invoiceNumber}. Select the line items you want
            to credit.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-6 py-4">
          {/* Line Items Selection */}
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <Label className="text-sm font-medium">Line Items</Label>
              <Button variant="ghost" size="sm" onClick={handleSelectAll}>
                {lineItems.every(item => item.selected) ? 'Deselect All' : 'Select All'}
              </Button>
            </div>

            <div className="border rounded-md divide-y">
              {lineItems.map(item => (
                <div key={item.lineItemLocalId} className="p-3 space-y-2">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3">
                      <Checkbox
                        checked={item.selected}
                        onCheckedChange={() => handleToggleLineItem(item.lineItemLocalId)}
                      />
                      <span className="text-sm font-medium">{item.name}</span>
                    </div>
                    <span className="text-sm text-muted-foreground">
                      Max: {formatCurrency(item.maxAmount, invoice.currency)}
                    </span>
                  </div>

                  {item.selected && (
                    <div className="ml-7 flex items-center gap-2">
                      <Label className="text-xs text-muted-foreground whitespace-nowrap">
                        Credit amount:
                      </Label>
                      <Input
                        type="number"
                        step="0.01"
                        min="0"
                        max={(item.maxAmount / 100).toFixed(2)}
                        placeholder={`Full amount (${(item.maxAmount / 100).toFixed(2)})`}
                        value={item.amount}
                        onChange={e => handleAmountChange(item.lineItemLocalId, e.target.value)}
                        className="w-40 h-8"
                      />
                      <span className="text-xs text-muted-foreground">{invoice.currency}</span>
                    </div>
                  )}
                </div>
              ))}
            </div>

            {selectedCount > 0 && (
              <div className="text-sm text-right">
                Total credit (excl. tax):{' '}
                <span className="font-semibold">
                  {formatCurrency(totalCreditAmount, invoice.currency)}
                </span>
              </div>
            )}
          </div>

          {/* Credit Type */}
          <div className="space-y-3">
            <Label className="text-sm font-medium">Credit Type</Label>
            <RadioGroup
              value={creditType.toString()}
              onValueChange={value => setCreditType(parseInt(value) as CreditType)}
            >
              <div className="flex items-center space-x-2">
                <RadioGroupItem
                  value={CreditType.CREDIT_TO_BALANCE.toString()}
                  id="credit-balance"
                />
                <Label htmlFor="credit-balance" className="text-sm font-normal cursor-pointer">
                  Credit to customer balance
                  <span className="block text-xs text-muted-foreground">
                    Amount will be applied to future invoices
                  </span>
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value={CreditType.REFUND.toString()} id="credit-refund" />
                <Label htmlFor="credit-refund" className="text-sm font-normal cursor-pointer">
                  Refund
                  <span className="block text-xs text-warning">
                    Paid amount must be refunded via your payment provider{' '}
                    <b>(NOT yet handled by Meteroid)</b>.
                  </span>
                  <span className="block text-xs text-muted-foreground">
                    Credit used will be re-credited to customer balance.
                  </span>
                </Label>
              </div>
            </RadioGroup>
          </div>

          {/* Reason */}
          <div className="space-y-2">
            <Label htmlFor="reason" className="text-sm font-medium">
              Reason (optional)
              <span className="block text-xs text-muted-foreground">
                (displayed on the credit note)
              </span>
            </Label>
            <Input
              id="reason"
              placeholder="e.g., Customer requested refund, Service not provided"
              value={reason}
              onChange={e => setReason(e.target.value)}
            />
          </div>

          {/* Memo */}
          <div className="space-y-2">
            <Label htmlFor="memo" className="text-sm font-medium">
              Memo (optional)
              <span className="block text-xs text-muted-foreground">
                (displayed on the credit note)
              </span>
            </Label>
            <Textarea
              id="memo"
              placeholder="Internal notes about this credit note..."
              value={memo}
              onChange={e => setMemo(e.target.value)}
              rows={2}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleClose}>
            Cancel
          </Button>
          <Button
            onClick={handleSubmit}
            disabled={selectedCount === 0 || createCreditNoteMutation.isPending}
          >
            {createCreditNoteMutation.isPending ? 'Creating...' : 'Create Credit Note'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
