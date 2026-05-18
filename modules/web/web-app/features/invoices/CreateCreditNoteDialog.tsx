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
import { DetailedInvoice, InvoicePaymentStatus, LineItem } from '@/rpc/api/invoices/v1/models_pb'

interface CreateCreditNoteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  invoice: DetailedInvoice
}

// Orthogonal axes replacing the old four-way radio:
//   scope       — partial (specific lines) vs full (entire invoice)
//   disposition — where the money goes (only asked when money has been paid)
//   reissue     — also open a corrected replacement draft (only for scope=full)
// The backend CreditType is *derived* from (scope, disposition, payment status).
type Scope = 'partial' | 'full'
type Disposition = 'reduce-debt' | 'refund' | 'credit-to-balance'

interface SubLineSelection {
  subLineLocalId: string
  name: string
  included: boolean
  quantity: string
  originalQuantity: string
  unitPrice: string
  originalUnitPrice: string
  originalTotal: number
}

interface LineItemSelection {
  lineItemLocalId: string
  selected: boolean
  quantity: string
  originalQuantity: string
  unitPrice: string | undefined
  originalUnitPrice: string | undefined
  maxAmount: number
  fullyCredited: boolean
  name: string
  subLines: SubLineSelection[]
}

const BarePriceInput: React.FC<{
  currency: string
  value: string
  placeholder?: string
  disabled?: boolean
  onChange: (value: string) => void
  className?: string
  inputClassName?: string
}> = ({ currency, value, placeholder, disabled, onChange, className, inputClassName }) => {
  const symbol = useMemo(() => {
    const f = new Intl.NumberFormat('en-US', { style: 'currency', currency, minimumFractionDigits: 2 })
    return f.format(0).replace(/\d|\./g, '').trim()
  }, [currency])
  return (
    <div className={`relative ${className ?? ''}`}>
      {symbol && (
        <div className="absolute inset-y-0 left-0 pl-2 flex items-center pointer-events-none">
          <span className="text-muted-foreground text-xs">{symbol}</span>
        </div>
      )}
      <input
        type="number"
        step="any"
        min="0"
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        onChange={e => onChange(e.target.value)}
        className={`pl-6 pr-10 bg-input block w-full border border-border rounded-md focus:outline-none focus:ring-1 focus:ring-ring placeholder:text-muted-foreground disabled:opacity-50 ${inputClassName ?? ''}`}
      />
      <div className="absolute inset-y-0 right-0 pr-2 flex items-center pointer-events-none">
        <span className="text-muted-foreground text-xs">{currency}</span>
      </div>
    </div>
  )
}

export const CreateCreditNoteDialog: React.FC<CreateCreditNoteDialogProps> = ({
  open,
  onOpenChange,
  invoice,
}) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const paymentStatus = invoice.paymentStatus
  const isPaid = paymentStatus === InvoicePaymentStatus.PAID
  const isPartiallyPaid = paymentStatus === InvoicePaymentStatus.PARTIALLY_PAID
  const isUnpaid = !isPaid && !isPartiallyPaid

  // Allowed dispositions per (scope, payment status):
  // - Unpaid:        reduce-debt only (trivial, no radio shown)
  // - PartiallyPaid: partial scope ⇒ reduce-debt | refund | credit-to-balance
  //                  full scope    ⇒ disabled (dual-CN primitive deferred)
  // - Paid:          refund | credit-to-balance (debt is settled; reduce-debt is meaningless)
  const fullScopeDisabled = isPartiallyPaid

  const allowedDispositionsForScope = (scope: Scope): Disposition[] => {
    if (isUnpaid) return ['reduce-debt']
    if (isPaid) return ['refund', 'credit-to-balance']
    // PartiallyPaid
    return scope === 'partial' ? ['reduce-debt', 'refund', 'credit-to-balance'] : []
  }

  const dispositionToCreditType = (d: Disposition): CreditType => {
    switch (d) {
      case 'reduce-debt':
        return CreditType.DEBT_CANCELLATION
      case 'refund':
        return CreditType.REFUND
      case 'credit-to-balance':
        return CreditType.CREDIT_TO_BALANCE
    }
  }

  // Fetch existing CNs to show per-line remaining amounts (and to avoid double-crediting).
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
        originalUnitPrice: item.unitPrice,
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
          originalUnitPrice: sl.unitPrice,
          originalTotal: Math.abs(Number(sl.total)),
        })),
      }
    })
  }, [invoice.lineItems, existingCreditNotes])

  const [lineItems, setLineItems] = useState<LineItemSelection[]>(initialLineItems)
  useEffect(() => {
    setLineItems(initialLineItems)
  }, [initialLineItems])

  const defaultScope: Scope = 'partial'
  const [scope, setScope] = useState<Scope>(defaultScope)
  const defaultDispositionFor = (s: Scope): Disposition =>
    allowedDispositionsForScope(s)[0] ?? 'reduce-debt'
  const [disposition, setDisposition] = useState<Disposition>(defaultDispositionFor(defaultScope))
  const [reissue, setReissue] = useState(false)
  const [reason, setReason] = useState('')
  const [memo, setMemo] = useState('')

  // Keep disposition in a valid state as scope changes.
  useEffect(() => {
    const allowed = allowedDispositionsForScope(scope)
    if (!allowed.includes(disposition)) {
      setDisposition(allowed[0] ?? 'reduce-debt')
    }
    if (scope !== 'full' && reissue) {
      setReissue(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope])

  const createCreditNoteMutation = useMutation(createCreditNote, {
    onSuccess: async data => {
      toast.success(reissue ? 'Invoice cancelled and reissued' : 'Credit note created')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getInvoice, { id: invoice.id }),
      })
      onOpenChange(false)
      if (data.correctedInvoiceId) {
        navigate(`${basePath}/invoices/${data.correctedInvoiceId}`)
      } else if (data.creditNote?.id) {
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

  const handleUnitPriceChange = (localId: string, value: string) => {
    setLineItems(prev =>
      prev.map(item => (item.lineItemLocalId === localId ? { ...item, unitPrice: value } : item))
    )
  }

  const handleSubLineUnitPriceChange = (
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
                sl.subLineLocalId === subLocalId ? { ...sl, unitPrice: value } : sl
              ),
            }
          : item
      )
    )
  }

  const handleSubLineQuantityChange = (lineLocalId: string, subLocalId: string, value: string) => {
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

  const precision = CURRENCIES[invoice.currency]?.precision ?? 2
  const pow10 = new Decimal(10).pow(precision)
  const toSubunit = (amount: Decimal): Decimal =>
    amount.times(pow10).toDecimalPlaces(0, Decimal.ROUND_HALF_UP)

  const computeLineCreditSubunit = (item: LineItemSelection): number => {
    if (item.subLines.length > 0) {
      const total = item.subLines.reduce((sum, sl) => {
        if (!sl.included) return sum
        const q = parseDecimal(sl.quantity)
        const up = parseDecimal(sl.unitPrice)
        if (!q || q.lte(0) || !up || up.lte(0)) return sum
        return sum.plus(toSubunit(up.times(q)))
      }, new Decimal(0))
      return total.toNumber()
    }
    if (!item.unitPrice) return 0
    const up = parseDecimal(item.unitPrice)
    if (!up || up.lte(0)) return 0
    const q = item.quantity.trim() === '' ? parseDecimal(item.originalQuantity) : parseDecimal(item.quantity)
    if (!q || q.lte(0)) return 0
    return toSubunit(up.times(q)).toNumber()
  }

  const handleSelectAll = () => {
    const allSelected = lineItems.every(item => item.selected)
    setLineItems(prev => prev.map(item => ({ ...item, selected: !allSelected })))
  }

  const handleSubmit = () => {
    if (scope === 'full' && fullScopeDisabled) {
      toast.error(
        'Cancelling a partially paid invoice is not supported yet. Collect or refund the paid portion first.'
      )
      return
    }

    // scope=full: empty line_items means "credit everything" on the backend.
    // scope=partial: validate selection and build explicit line items.
    let creditLineItems: Array<{
      lineItemLocalId: string
      quantity?: string
      unitPrice?: string
      subLines: Array<{ subLineLocalId: string; quantity: string; unitPrice?: string }>
    }> = []

    if (scope === 'partial') {
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
          for (const sl of kept) {
            const up = parseDecimal(sl.unitPrice)
            if (!up || up.lte(0)) {
              toast.error(`"${item.name} / ${sl.name}": unit price must be positive`)
              return
            }
            const origUp = parseDecimal(sl.originalUnitPrice)
            if (origUp && up.gt(origUp)) {
              toast.error(
                `"${item.name} / ${sl.name}": unit price exceeds original (${sl.originalUnitPrice})`
              )
              return
            }
          }
        } else {
          if (item.quantity.trim() !== '') {
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
          if (item.unitPrice !== undefined && item.unitPrice.trim() !== '') {
            const up = parseDecimal(item.unitPrice)
            if (!up || up.lte(0)) {
              toast.error(`"${item.name}": unit price must be positive`)
              return
            }
            const origUp = parseDecimal(item.originalUnitPrice ?? '')
            if (origUp && up.gt(origUp)) {
              toast.error(
                `"${item.name}": unit price exceeds original (${item.originalUnitPrice})`
              )
              return
            }
          }
        }
      }

      creditLineItems = selectedItems.map(item => {
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
              unitPrice: sl.unitPrice !== sl.originalUnitPrice ? sl.unitPrice : undefined,
            }))
          return {
            lineItemLocalId: item.lineItemLocalId,
            quantity: undefined,
            subLines,
          }
        }
        const unitPriceChanged =
          item.unitPrice !== undefined &&
          item.unitPrice.trim() !== '' &&
          item.unitPrice !== item.originalUnitPrice
        return {
          lineItemLocalId: item.lineItemLocalId,
          quantity: item.quantity.trim() === '' ? undefined : item.quantity,
          unitPrice: unitPriceChanged ? item.unitPrice : undefined,
          subLines: [],
        }
      })
    }

    createCreditNoteMutation.mutate({
      creditNote: {
        invoiceId: invoice.id,
        lineItems: creditLineItems,
        reason: reason || undefined,
        memo: memo || undefined,
        creditType: dispositionToCreditType(disposition),
      },
      finalize: true,
      reissueAsDraft: scope === 'full' && reissue,
    })
  }

  const selectedCount = lineItems.filter(item => item.selected).length
  const totalCreditAmount = lineItems
    .filter(item => item.selected)
    .reduce((sum, item) => sum + computeLineCreditSubunit(item), 0)

  const handleClose = () => {
    setLineItems(initialLineItems)
    setScope(defaultScope)
    setDisposition(defaultDispositionFor(defaultScope))
    setReissue(false)
    setReason('')
    setMemo('')
    onOpenChange(false)
  }

  const dispositionsNow = allowedDispositionsForScope(scope)
  const showDispositionRadio = dispositionsNow.length > 1

  const submitLabel = createCreditNoteMutation.isPending
    ? reissue
      ? 'Cancelling & reissuing...'
      : 'Creating...'
    : reissue
      ? 'Cancel and reissue'
      : 'Create credit note'

  const submitDisabled =
    createCreditNoteMutation.isPending ||
    (scope === 'full' && fullScopeDisabled) ||
    (scope === 'partial' && selectedCount === 0)

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Credit note — {invoice.invoiceNumber}</DialogTitle>
          <DialogDescription>
            Issue a credit note against this invoice. Choose the scope, then how the money should be
            handled.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-6 py-4">
          {/* Scope */}
          <div className="space-y-3">
            <Label className="text-sm font-medium">Scope</Label>
            <RadioGroup value={scope} onValueChange={v => setScope(v as Scope)}>
              <div className="flex items-start space-x-2">
                <RadioGroupItem value="partial" id="scope-partial" className="mt-1" />
                <Label htmlFor="scope-partial" className="text-sm font-normal cursor-pointer">
                  Credit specific lines or amounts
                  <span className="block text-xs text-muted-foreground">
                    Select which lines to credit. The invoice stays open for the remaining balance.
                  </span>
                </Label>
              </div>
              <div className="flex items-start space-x-2">
                <RadioGroupItem
                  value="full"
                  id="scope-full"
                  className="mt-1"
                  disabled={fullScopeDisabled}
                />
                <Label
                  htmlFor="scope-full"
                  className={`text-sm font-normal ${fullScopeDisabled ? 'opacity-50' : 'cursor-pointer'}`}
                >
                  Cancel the entire invoice
                  <span className="block text-xs text-muted-foreground">
                    {fullScopeDisabled ? (
                      'Not supported for partially paid invoices. Collect or refund the paid portion first.'
                    ) : (
                      <>
                        Credits the full amount. The cancelled invoice is kept on record as required
                        by law. <br /> You will be able to issue a corrected invoice.
                      </>
                    )}
                  </span>
                </Label>
              </div>
            </RadioGroup>
          </div>

          {/* Disposition (only when there is a real choice) */}
          {showDispositionRadio && (
            <div className="space-y-3">
              <Label className="text-sm font-medium">Where should the money go?</Label>
              <RadioGroup value={disposition} onValueChange={v => setDisposition(v as Disposition)}>
                {dispositionsNow.includes('reduce-debt') && (
                  <div className="flex items-start space-x-2">
                    <RadioGroupItem value="reduce-debt" id="disp-debt" className="mt-1" />
                    <Label htmlFor="disp-debt" className="text-sm font-normal cursor-pointer">
                      Reduce the amount due
                      <span className="block text-xs text-muted-foreground">
                        The credit is applied against the unpaid portion of this invoice.
                      </span>
                    </Label>
                  </div>
                )}
                {dispositionsNow.includes('refund') && (
                  <div className="flex items-start space-x-2">
                    <RadioGroupItem value="refund" id="disp-refund" className="mt-1" />
                    <Label htmlFor="disp-refund" className="text-sm font-normal cursor-pointer">
                      Refund to customer
                      <span className="block text-xs text-muted-foreground">
                        The paid amount must be refunded to the customer via your payment provider
                        (out of band). Any applied customer credit is restored to the balance.
                      </span>
                    </Label>
                  </div>
                )}
                {dispositionsNow.includes('credit-to-balance') && (
                  <div className="flex items-start space-x-2">
                    <RadioGroupItem value="credit-to-balance" id="disp-ctb" className="mt-1" />
                    <Label htmlFor="disp-ctb" className="text-sm font-normal cursor-pointer">
                      Apply as customer credit
                      <span className="block text-xs text-muted-foreground">
                        The amount is added to the customer&apos;s balance and applied to future
                        invoices.
                      </span>
                    </Label>
                  </div>
                )}
              </RadioGroup>
            </div>
          )}

          {/* Full-scope summary */}
          {scope === 'full' && !fullScopeDisabled && (
            <div className="rounded-md border border-border bg-muted/30 p-4 text-sm space-y-1">
              <div>
                The full invoice amount (
                <span className="font-medium">
                  {formatCurrency(Number(invoice.total), invoice.currency)}
                </span>
                ) will be credited.
              </div>
              {isUnpaid && (
                <div className="text-xs text-muted-foreground">
                  The invoice will be marked as fully settled.
                </div>
              )}
            </div>
          )}

          {/* Partial-scope line item selector */}
          {scope === 'partial' && (
            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <Label className="text-sm font-medium">Line items</Label>
                <Button variant="ghost" size="sm" onClick={handleSelectAll}>
                  {lineItems.every(item => item.selected) ? 'Deselect all' : 'Select all'}
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
                      <div className="ml-7 flex items-center gap-2 flex-wrap">
                        <Input
                          type="number"
                          step="any"
                          min="0"
                          max={item.originalQuantity}
                          placeholder={item.originalQuantity}
                          value={item.quantity}
                          onChange={e => handleQuantityChange(item.lineItemLocalId, e.target.value)}
                          className="w-20 h-8"
                        />
                        <span className="text-xs text-muted-foreground">×</span>
                        <BarePriceInput
                          currency={invoice.currency}
                          placeholder={item.originalUnitPrice ?? ''}
                          value={item.unitPrice ?? ''}
                          onChange={v => handleUnitPriceChange(item.lineItemLocalId, v)}
                          className="w-36"
                          inputClassName="h-8 text-sm"
                        />
                        <span className="text-xs text-muted-foreground whitespace-nowrap">
                          = {formatCurrency(computeLineCreditSubunit(item), invoice.currency)}
                        </span>
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
                              className="w-20 h-7 text-xs"
                            />
                            <span className="text-xs text-muted-foreground">×</span>
                            <BarePriceInput
                              currency={invoice.currency}
                              placeholder={sl.originalUnitPrice}
                              value={sl.unitPrice}
                              disabled={!sl.included}
                              onChange={v =>
                                handleSubLineUnitPriceChange(
                                  item.lineItemLocalId,
                                  sl.subLineLocalId,
                                  v
                                )
                              }
                              className="w-32"
                              inputClassName="h-7 text-xs"
                            />
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
          )}

          {/* Reissue checkbox (full scope only) */}
          {scope === 'full' && !fullScopeDisabled && (
            <div className="flex items-start gap-2">
              <Checkbox
                id="reissue"
                checked={reissue}
                onCheckedChange={v => setReissue(v === true)}
                className="mt-0.5"
              />
              <Label htmlFor="reissue" className="text-sm font-normal cursor-pointer">
                Also open a corrected draft invoice to reissue
                <span className="block text-xs text-muted-foreground">
                  Creates an editable draft copy so you can fix the line items before reissuing. The
                  cancelled invoice is kept on record.
                </span>
              </Label>
            </div>
          )}

          {/* Reason */}
          <div className="space-y-2">
            <Label htmlFor="reason" className="text-sm font-medium">
              Reason (optional)
              <span className="block text-xs text-muted-foreground">
                Displayed on the credit note (visible to customer).
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
                Internal note (not visible to customer).
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
          <Button onClick={handleSubmit} disabled={submitDisabled}>
            {submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
