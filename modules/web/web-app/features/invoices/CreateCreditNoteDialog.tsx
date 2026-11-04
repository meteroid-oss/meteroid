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
  Form,
  Input,
  Label,
  RadioGroup,
  RadioGroupItem,
  Textarea,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import Decimal from 'decimal.js'
import { useEffect, useMemo, useState } from 'react'
import { useForm } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import {
  CustomPropertiesFields,
  serializeCustomProperties,
} from '@/features/customproperties/CustomPropertiesFields'
import {
  Disposition,
  FullScopeBlocker,
  Scope,
  allowedDispositions,
  creditToBalanceBlocked,
  estimatedNoteTotal,
  exceedsAmountDue,
  fullScopeBlocker,
  quantityError,
  reissueBlocked,
  sellsCredit,
} from '@/features/invoices/creditNoteRules'
import { useBasePath } from '@/hooks/useBasePath'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrency } from '@/lib/utils/numbers'
import {
  createCreditNote,
  listCreditNotesByInvoiceId,
} from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import { CreditNoteStatus, CreditType } from '@/rpc/api/creditnotes/v1/models_pb'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
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

interface SubLineSelection {
  subLineLocalId: string
  name: string
  included: boolean
  quantity: string
  originalQuantity: string
  unitPrice: string
  originalTotal: number
}

interface LineItemSelection {
  lineItemLocalId: string
  selected: boolean
  quantity: string
  originalQuantity: string
  unitPrice: string | undefined
  maxAmount: number
  fullyCredited: boolean
  name: string
  subLines: SubLineSelection[]
  // Usage priced in a credit unit: only its money overage is credited, by amount.
  creditUnitCode: string | undefined
  coveredByCredits: boolean
  amount: string
}

export const CreateCreditNoteDialog: React.FC<CreateCreditNoteDialogProps> = ({
  open,
  onOpenChange,
  invoice,
}) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const isUnpaid =
    invoice.paymentStatus !== InvoicePaymentStatus.PAID &&
    invoice.paymentStatus !== InvoicePaymentStatus.PARTIALLY_PAID

  const fullScopeBlockedBy = fullScopeBlocker(invoice)
  const fullScopeDisabled = fullScopeBlockedBy !== null
  const allowedDispositionsForScope = (scope: Scope): Disposition[] =>
    allowedDispositions(invoice, scope)

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
        maxAmount: remaining,
        fullyCredited: remaining === 0,
        name: item.name,
        creditUnitCode: item.creditUnit?.code,
        coveredByCredits: item.creditUnit !== undefined && original === 0,
        amount: '',
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

  // Custom properties use react-hook-form (what CustomPropertiesFields expects); the rest of this
  // dialog is plain useState. We only read the bag on submit, so the two coexist fine.
  const customPropsForm = useForm<{ customProperties: Record<string, unknown> }>({
    defaultValues: { customProperties: {} },
  })

  const creditedLines = useMemo(() => {
    if (scope === 'full') return invoice.lineItems
    const selected = new Set(
      lineItems.filter(item => item.selected).map(item => item.lineItemLocalId)
    )
    return invoice.lineItems.filter(item => selected.has(item.id))
  }, [scope, lineItems, invoice.lineItems])
  const ctbBlocked = creditToBalanceBlocked(creditedLines)
  const refundOfSoldCredit = disposition === 'refund' && creditedLines.some(sellsCredit)
  const reissueDisabled = reissueBlocked(scope, disposition, creditedLines)

  // Keep disposition and reissue in a valid state as scope and selection change.
  useEffect(() => {
    const allowed = allowedDispositionsForScope(scope).filter(
      d => !(d === 'credit-to-balance' && ctbBlocked)
    )
    if (!allowed.includes(disposition)) {
      setDisposition(allowed[0] ?? 'reduce-debt')
    }
    if ((scope !== 'full' || reissueDisabled) && reissue) {
      setReissue(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, ctbBlocked, reissueDisabled])

  const createCreditNoteMutation = useMutation(createCreditNote, {
    onSuccess: async data => {
      toast.success(reissue ? 'Invoice cancelled and reissued' : 'Credit note created')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoice,
          input: { id: invoice.id },
          cardinality: 'finite',
        }),
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

  const handleAmountChange = (localId: string, value: string) => {
    setLineItems(prev =>
      prev.map(item => (item.lineItemLocalId === localId ? { ...item, amount: value } : item))
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

  const precision = currencyPrecision(invoice.currency)
  const pow10 = new Decimal(10).pow(precision)
  const toSubunit = (amount: Decimal): Decimal =>
    amount.times(pow10).toDecimalPlaces(0, Decimal.ROUND_HALF_UP)

  const computeLineCreditSubunit = (item: LineItemSelection): number => {
    if (item.creditUnitCode !== undefined) {
      if (item.amount.trim() === '') return item.maxAmount
      const a = parseDecimal(item.amount)
      return a && a.gt(0) ? toSubunit(a).toNumber() : 0
    }
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
    if (scope === 'full' && fullScopeBlockedBy !== null) {
      toast.error(fullScopeBlockerText(fullScopeBlockedBy, invoice))
      return
    }

    // scope=full: empty line_items means "credit everything" on the backend.
    // scope=partial: validate selection and build explicit line items.
    let creditLineItems: Array<{
      lineItemLocalId: string
      quantity?: string
      amount?: bigint
      subLines: Array<{ subLineLocalId: string; quantity: string }>
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
        if (item.creditUnitCode !== undefined) {
          const credited = computeLineCreditSubunit(item)
          if (credited <= 0) {
            toast.error(`"${item.name}": amount must be a positive number`)
            return
          }
          if (credited > item.maxAmount) {
            toast.error(
              `"${item.name}": amount exceeds the ${formatCurrency(item.maxAmount, invoice.currency)} left to credit`
            )
            return
          }
        } else if (item.subLines.length > 0) {
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
        } else {
          const error = lineQuantityError(item)
          if (error) {
            toast.error(`"${item.name}": ${error}`)
            return
          }
        }
      }
      if (exceedsAmountDue(disposition, estimatedTotal, invoice)) {
        toast.error(amountDueText)
        return
      }

      creditLineItems = selectedItems.map(item => {
        if (item.creditUnitCode !== undefined) {
          return {
            lineItemLocalId: item.lineItemLocalId,
            amount: BigInt(computeLineCreditSubunit(item)),
            subLines: [],
          }
        }
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
    }

    createCreditNoteMutation.mutate({
      creditNote: {
        invoiceId: invoice.id,
        lineItems: creditLineItems,
        reason: reason || undefined,
        memo: memo || undefined,
        creditType: dispositionToCreditType(disposition),
        customProperties: serializeCustomProperties(customPropsForm.getValues('customProperties')),
      },
      finalize: true,
      reissueAsDraft: scope === 'full' && reissue,
    })
  }

  const lineQuantityError = (item: LineItemSelection): string | null => {
    if (item.creditUnitCode !== undefined || item.subLines.length > 0) return null
    const error = quantityError(
      item.quantity,
      item.originalQuantity,
      computeLineCreditSubunit(item),
      item.maxAmount
    )
    switch (error?.kind) {
      case undefined:
        return null
      case 'not-positive':
        return 'quantity must be a positive number'
      case 'above-original':
        return `at most ${error.original}, the quantity invoiced`
      case 'above-remaining':
        return `exceeds the ${formatCurrency(item.maxAmount, invoice.currency)} left to credit`
    }
  }

  const selectedCount = lineItems.filter(item => item.selected).length
  const totalCreditAmount = lineItems
    .filter(item => item.selected)
    .reduce((sum, item) => sum + computeLineCreditSubunit(item), 0)
  const lineById = new Map(invoice.lineItems.map(line => [line.id, line]))
  // What the selection credits tax included, as the server prorates each line.
  const estimatedTotal = lineItems
    .filter(item => item.selected)
    .reduce((sum, item) => {
      const line = lineById.get(item.lineItemLocalId)
      return line ? sum + estimatedNoteTotal(line, computeLineCreditSubunit(item)) : sum
    }, 0n)
  const hasQuantityError = lineItems.some(item => item.selected && lineQuantityError(item) !== null)
  const aboveAmountDue =
    scope === 'partial' && exceedsAmountDue(disposition, estimatedTotal, invoice)
  const amountDueText = `A debt cancellation can cancel at most the ${formatCurrency(Number(invoice.amountDue), invoice.currency)} still due, tax included.`

  const handleClose = () => {
    setLineItems(initialLineItems)
    setScope(defaultScope)
    setDisposition(defaultDispositionFor(defaultScope))
    setReissue(false)
    setReason('')
    setMemo('')
    customPropsForm.reset({ customProperties: {} })
    onOpenChange(false)
  }

  const dispositionsNow = allowedDispositionsForScope(scope)
  // A single choice is still shown when it is not the obvious one (reducing what is due).
  const showDispositionRadio =
    dispositionsNow.length > 1 ||
    (dispositionsNow.length === 1 && dispositionsNow[0] !== 'reduce-debt')
  const noDisposition =
    dispositionsNow.length > 0 &&
    dispositionsNow.every(d => d === 'credit-to-balance' && ctbBlocked)

  const submitLabel = createCreditNoteMutation.isPending
    ? reissue
      ? 'Cancelling & reissuing...'
      : 'Creating...'
    : reissue
      ? 'Cancel and reissue'
      : 'Create credit note'

  const submitDisabled =
    createCreditNoteMutation.isPending ||
    noDisposition ||
    (scope === 'full' && fullScopeDisabled) ||
    (scope === 'partial' && (selectedCount === 0 || hasQuantityError || aboveAmountDue))

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

        <Form {...customPropsForm}>
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
                      Select which lines to credit. The invoice stays open for the remaining
                      balance.
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
                      {fullScopeBlockedBy !== null ? (
                        fullScopeBlockerText(fullScopeBlockedBy, invoice)
                      ) : (
                        <>
                          Credits the full amount. The cancelled invoice is kept on record as
                          required by law. <br /> You will be able to issue a corrected invoice.
                        </>
                      )}
                    </span>
                  </Label>
                </div>
              </RadioGroup>
            </div>

            {noDisposition && (
              <div className="text-xs text-warning">
                A payment is in progress. Purchased credit can be credited once it settles, with a
                refund.
              </div>
            )}

            {/* Disposition (only when there is a real choice) */}
            {showDispositionRadio && (
              <div className="space-y-3">
                <Label className="text-sm font-medium">Where should the money go?</Label>
                <RadioGroup
                  value={disposition}
                  onValueChange={v => setDisposition(v as Disposition)}
                >
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
                          Creates a refundable credit note; the refund itself is issued from the
                          credit note once finalized. Any applied customer credit is restored to the
                          balance.
                        </span>
                        {refundOfSoldCredit && (
                          <span className="block text-xs text-warning">
                            Purchased credit is refunded only up to what is still unspent: the note
                            may credit less than shown below.
                          </span>
                        )}
                      </Label>
                    </div>
                  )}
                  {dispositionsNow.includes('credit-to-balance') && (
                    <div className="flex items-start space-x-2">
                      <RadioGroupItem
                        value="credit-to-balance"
                        id="disp-ctb"
                        className="mt-1"
                        disabled={ctbBlocked}
                      />
                      <Label
                        htmlFor="disp-ctb"
                        className={`text-sm font-normal ${ctbBlocked ? 'opacity-50' : 'cursor-pointer'}`}
                      >
                        Apply as customer credit
                        <span className="block text-xs text-muted-foreground">
                          {ctbBlocked
                            ? 'Purchased credit is reversed with a refund.'
                            : "The amount is added to the customer's balance and applied to future invoices."}
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
                  ) will be credited
                  {refundOfSoldCredit ? ', less any purchased credit already spent.' : '.'}
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
                          {item.coveredByCredits ? (
                            <span className="text-xs text-muted-foreground">
                              (covered by {item.creditUnitCode} credits)
                            </span>
                          ) : (
                            item.fullyCredited && (
                              <span className="text-xs text-muted-foreground">
                                (fully credited)
                              </span>
                            )
                          )}
                        </div>
                        <span className="text-sm text-muted-foreground">
                          Remaining: {formatCurrency(item.maxAmount, invoice.currency)}
                        </span>
                      </div>

                      {item.creditUnitCode !== undefined &&
                        (item.selected || item.coveredByCredits) && (
                          <div className="ml-7 space-y-1.5">
                            <p className="text-xs text-muted-foreground">
                              Priced in {item.creditUnitCode} credits: a credit note credits only
                              the money billed for credits the customer&apos;s balance did not
                              cover. Credits come back by voiding the invoice or by a manual grant.
                            </p>
                            {item.selected && (
                              <div className="flex items-center gap-2">
                                <Label className="text-xs text-muted-foreground whitespace-nowrap">
                                  Credit amount:
                                </Label>
                                <Input
                                  type="number"
                                  step="any"
                                  min="0"
                                  placeholder={`Full (${new Decimal(item.maxAmount).div(pow10).toFixed(precision)})`}
                                  value={item.amount}
                                  onChange={e =>
                                    handleAmountChange(item.lineItemLocalId, e.target.value)
                                  }
                                  className="w-32 h-8"
                                />
                                <span className="text-xs text-muted-foreground">
                                  {invoice.currency}
                                </span>
                              </div>
                            )}
                          </div>
                        )}

                      {item.selected &&
                        item.creditUnitCode === undefined &&
                        item.subLines.length === 0 && (
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
                            {lineQuantityError(item) && (
                              <span className="text-xs text-destructive">
                                {lineQuantityError(item)}
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
                  <div className="text-right space-y-0.5">
                    <div className="text-sm">
                      Credited before discounts, promotional credit and tax:{' '}
                      <span className="font-semibold">
                        {formatCurrency(totalCreditAmount, invoice.currency)}
                      </span>
                    </div>
                    <div className="text-xs text-muted-foreground">
                      About {formatCurrency(Number(estimatedTotal), invoice.currency)} tax included,
                      once each line&apos;s share of any discount or promotional credit is taken
                      off.
                    </div>
                    {aboveAmountDue && (
                      <div className="text-xs text-destructive">{amountDueText}</div>
                    )}
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
                  disabled={reissueDisabled}
                  onCheckedChange={v => setReissue(v === true)}
                  className="mt-0.5"
                />
                <Label
                  htmlFor="reissue"
                  className={`text-sm font-normal ${reissueDisabled ? 'opacity-50' : 'cursor-pointer'}`}
                >
                  Also open a corrected draft invoice to reissue
                  <span className="block text-xs text-muted-foreground">
                    {reissueDisabled
                      ? 'Unavailable when refunding purchased credit: the refund stops at the credit still unspent, so the invoice may not be fully credited. Create the corrected invoice afterwards.'
                      : 'Creates an editable draft copy so you can fix the line items before reissuing. The cancelled invoice is kept on record.'}
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

            {/* Custom properties (only renders if the tenant defined any for credit notes) */}
            <CustomPropertiesFields
              control={customPropsForm.control}
              entityType={CustomPropertyEntityType.CREDIT_NOTE}
            />
          </div>
        </Form>

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

const fullScopeBlockerText = (
  blocker: FullScopeBlocker,
  invoice: Pick<DetailedInvoice, 'amountDue' | 'currency'>
): string => {
  switch (blocker) {
    case 'partially-paid':
      return 'Not supported for partially paid invoices. Collect or refund the paid portion first.'
    case 'credit-paid-part':
      return `Customer credit paid part of this invoice. Void the invoice to cancel it, or credit specific lines up to the ${formatCurrency(Number(invoice.amountDue), invoice.currency)} still due.`
  }
}
