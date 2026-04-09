import { createConnectQueryKey, useMutation, useQuery } from '@connectrpc/connect-query'
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
import Decimal from 'decimal.js'
import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { useBasePath } from '@/hooks/useBasePath'
import { CURRENCIES } from '@/lib/data/currencies'
import { formatCurrency } from '@/lib/utils/numbers'
import {
  createCreditNote,
  listCreditNotesByInvoiceId,
} from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import { CreditNoteStatus, CreditType } from '@/rpc/api/creditnotes/v1/models_pb'
import { getInvoice } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { DetailedInvoice, LineItem } from '@/rpc/api/invoices/v1/models_pb'


interface CreateCreditNoteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  invoice: DetailedInvoice
}

interface SubLineSelection {
  subLineLocalId: string
  name: string
  included: boolean
  quantity: string // user input (decimal string), prefilled with originalQuantity
  originalQuantity: string
  unitPrice: string
  originalTotal: number // cents, for display
}

interface LineItemSelection {
  lineItemLocalId: string
  selected: boolean
  quantity: string // used when no sublines
  originalQuantity: string
  unitPrice: string | undefined
  maxAmount: number
  fullyCredited: boolean
  name: string
  subLines: SubLineSelection[]
}

export const CreateCreditNoteDialog: React.FC<CreateCreditNoteDialogProps> = ({
  open,
  onOpenChange,
  invoice,
}) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  // Fetch existing credit notes for this invoice to compute per-line remaining amounts.
  // Without this, the UI would show the original subtotal as the cap even when the line
  // was already partially credited, and the backend would silently cap the difference.
  const existingCreditNotesQuery = useQuery(
    listCreditNotesByInvoiceId,
    { invoiceId: invoice.id },
    { enabled: open && Boolean(invoice.id) }
  )

  const existingCreditNotes = existingCreditNotesQuery.data?.creditNotes

  const initialLineItems = useMemo<LineItemSelection[]>(() => {
    const alreadyByLine: Record<string, number> = {}
    for (const cn of existingCreditNotes ?? []) {
      if (cn.status === CreditNoteStatus.VOIDED) continue
      for (const li of cn.lineItems) {
        alreadyByLine[li.id] = (alreadyByLine[li.id] ?? 0) + Math.abs(Number(li.subtotal))
      }
    }
    return invoice.lineItems.map((item: LineItem) => {
      const original = Math.abs(Number(item.subtotal))
      const already = alreadyByLine[item.id] ?? 0
      const remaining = Math.max(0, original - already)
      return {
        lineItemLocalId: item.id,
        selected: false,
        quantity: item.quantity ?? '1',
        originalQuantity: item.quantity ?? '1',
        unitPrice: item.unitPrice,
        maxAmount: remaining,
        fullyCredited: remaining === 0,
        name: item.name,
        subLines: item.subLineItems.map(sl => ({
          subLineLocalId: sl.id,
          name: sl.name,
          included: true,
          quantity: sl.quantity,
          originalQuantity: sl.quantity,
          unitPrice: sl.unitPrice,
          originalTotal: Math.abs(Number(sl.total)),
        })),
      }
    })
  }, [invoice.lineItems, existingCreditNotes])

  const [lineItems, setLineItems] = useState<LineItemSelection[]>(initialLineItems)

  // Reset local selection state when the underlying invoice or existing credit notes change
  // (e.g. query finishes loading after dialog opens).
  useEffect(() => {
    setLineItems(initialLineItems)
  }, [initialLineItems])
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

  const handleQuantityChange = (localId: string, value: string) => {
    setLineItems(prev =>
      prev.map(item => (item.lineItemLocalId === localId ? { ...item, quantity: value } : item))
    )
  }

  const handleSubLineQuantityChange = (
    lineLocalId: string,
    subLocalId: string,
    value: string
  ) => {
    setLineItems(prev =>
      prev.map(item =>
        item.lineItemLocalId === lineLocalId
          ? {
              ...item,
              subLines: item.subLines.map(sl =>
                sl.subLineLocalId === subLocalId ? { ...sl, quantity: value } : sl
              ),
            }
          : item
      )
    )
  }

  const handleToggleSubLine = (lineLocalId: string, subLocalId: string) => {
    setLineItems(prev =>
      prev.map(item =>
        item.lineItemLocalId === lineLocalId
          ? {
              ...item,
              subLines: item.subLines.map(sl =>
                sl.subLineLocalId === subLocalId ? { ...sl, included: !sl.included } : sl
              ),
            }
          : item
      )
    )
  }

  // Parse a user-entered decimal; returns null on invalid/empty.
  const parseDecimal = (s: string): Decimal | null => {
    const t = s.trim()
    if (t === '') return null
    try {
      const d = new Decimal(t)
      return d.isFinite() ? d : null
    } catch {
      return null
    }
  }

  // Currency-aware: precision comes from CURRENCIES (e.g. JPY=0, USD=2, BHD=3).
  const precision = CURRENCIES[invoice.currency]?.precision ?? 2
  const pow10 = new Decimal(10).pow(precision)

  const toSubunit = (amount: Decimal): Decimal =>
    amount.times(pow10).toDecimalPlaces(0, Decimal.ROUND_HALF_UP)

  // Credited amount in minor units, mirroring backend (unit_price × quantity per row, summed).
  const computeLineCreditSubunit = (item: LineItemSelection): number => {
    if (item.subLines.length > 0) {
      const total = item.subLines.reduce((sum, sl) => {
        if (!sl.included) return sum
        const q = parseDecimal(sl.quantity)
        if (!q || q.lte(0)) return sum
        return sum.plus(toSubunit(new Decimal(sl.unitPrice).times(q)))
      }, new Decimal(0))
      return total.toNumber()
    }
    if (item.quantity.trim() === '') return item.maxAmount
    const q = parseDecimal(item.quantity)
    if (!q || q.lte(0) || !item.unitPrice) return 0
    return toSubunit(new Decimal(item.unitPrice).times(q)).toNumber()
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

    for (const item of selectedItems) {
      if (item.fullyCredited) {
        toast.error(`"${item.name}" has already been fully credited`)
        return
      }
      if (item.subLines.length > 0) {
        const kept = item.subLines.filter(sl => {
          if (!sl.included) return false
          const q = parseDecimal(sl.quantity)
          return q !== null && q.gt(0)
        })
        if (kept.length === 0) {
          toast.error(
            `"${item.name}": select at least one sub-line with a positive quantity, or deselect the line`
          )
          return
        }
      } else if (item.quantity.trim() !== '') {
        const q = parseDecimal(item.quantity)
        if (!q || q.lte(0)) {
          toast.error(`"${item.name}": quantity must be a positive number`)
          return
        }
        const maxQ = parseDecimal(item.originalQuantity)
        if (maxQ && q.gt(maxQ)) {
          toast.error(`"${item.name}": quantity exceeds original (${item.originalQuantity})`)
          return
        }
      }
    }

    const creditLineItems = selectedItems.map(item => {
      if (item.subLines.length > 0) {
        const subLines = item.subLines
          .filter(sl => {
            if (!sl.included) return false
            const q = parseDecimal(sl.quantity)
            return q !== null && q.gt(0)
          })
          .map(sl => ({
            subLineLocalId: sl.subLineLocalId,
            quantity: sl.quantity,
          }))
        return {
          lineItemLocalId: item.lineItemLocalId,
          quantity: undefined,
          subLines,
        }
      }
      return {
        lineItemLocalId: item.lineItemLocalId,
        quantity: item.quantity.trim() === '' ? undefined : item.quantity,
        subLines: [],
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
    .reduce((sum, item) => sum + computeLineCreditSubunit(item), 0)

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
                        disabled={item.fullyCredited}
                        onCheckedChange={() => handleToggleLineItem(item.lineItemLocalId)}
                      />
                      <span className="text-sm font-medium">{item.name}</span>
                      {item.fullyCredited && (
                        <span className="text-xs text-muted-foreground">(fully credited)</span>
                      )}
                    </div>
                    <span className="text-sm text-muted-foreground">
                      Remaining: {formatCurrency(item.maxAmount, invoice.currency)}
                    </span>
                  </div>

                  {item.selected && item.subLines.length === 0 && (
                    <div className="ml-7 flex items-center gap-2">
                      <Label className="text-xs text-muted-foreground whitespace-nowrap">
                        Credit quantity:
                      </Label>
                      <Input
                        type="number"
                        step="any"
                        min="0"
                        max={item.originalQuantity}
                        placeholder={`Full (${item.originalQuantity})`}
                        value={item.quantity}
                        onChange={e =>
                          handleQuantityChange(item.lineItemLocalId, e.target.value)
                        }
                        className="w-32 h-8"
                      />
                      {item.unitPrice && (
                        <span className="text-xs text-muted-foreground">
                          × {item.unitPrice} {invoice.currency} ={' '}
                          {formatCurrency(computeLineCreditSubunit(item), invoice.currency)}
                        </span>
                      )}
                    </div>
                  )}

                  {item.selected && item.subLines.length > 0 && (
                    <div className="ml-7 space-y-1.5 border-l pl-3">
                      <div className="text-xs text-muted-foreground">
                        Uncheck to exclude a sub-line, or edit its quantity:
                      </div>
                      {item.subLines.map(sl => (
                        <div
                          key={sl.subLineLocalId}
                          className={`flex items-center gap-2 ${sl.included ? '' : 'opacity-50'}`}
                        >
                          <Checkbox
                            checked={sl.included}
                            onCheckedChange={() =>
                              handleToggleSubLine(item.lineItemLocalId, sl.subLineLocalId)
                            }
                          />
                          <span className="text-xs flex-1 truncate">{sl.name}</span>
                          <Input
                            type="number"
                            step="any"
                            min="0"
                            max={sl.originalQuantity}
                            placeholder={sl.originalQuantity}
                            value={sl.quantity}
                            disabled={!sl.included}
                            onChange={e =>
                              handleSubLineQuantityChange(
                                item.lineItemLocalId,
                                sl.subLineLocalId,
                                e.target.value
                              )
                            }
                            className="w-24 h-7 text-xs"
                          />
                          <span className="text-xs text-muted-foreground whitespace-nowrap">
                            × {sl.unitPrice} {invoice.currency}
                          </span>
                        </div>
                      ))}
                      <div className="text-xs text-right">
                        Line credit:{' '}
                        <span className="font-medium">
                          {formatCurrency(computeLineCreditSubunit(item), invoice.currency)}
                        </span>
                      </div>
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
