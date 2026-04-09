import {
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  GenericFormField,
  Input,
  Label,
  Textarea,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { Info, Plus, Trash2 } from 'lucide-react'
import { useEffect } from 'react'
import { useFieldArray } from 'react-hook-form'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { DatePickerWithRange } from '@/features/dashboard/DateRangePicker'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import {
  UpdateInvoiceLineSchema,
  UpdateInvoiceLineSchemaWithSublines,
} from '@/lib/schemas/invoices'
import { formatCurrency, majorToMinorUnit } from '@/utils/numbers'

type LineModalValue = UpdateInvoiceLineSchema | UpdateInvoiceLineSchemaWithSublines

interface LineItemModalProps {
  isOpen: boolean
  onClose: () => void
  onSave: (item: LineModalValue) => void
  currency: string
  initialData?: LineModalValue
  isUsageBased?: boolean
  hasSublines?: boolean
}

export const LineItemModal = ({
  isOpen,
  onClose,
  onSave,
  currency,
  initialData,
  isUsageBased = false,
  hasSublines = false,
}: LineItemModalProps) => {
  const lineItemMethods = useZodForm({
    schema: hasSublines
      ? (schemas.invoices
          .updateInvoiceLineWithSublinesSchema as unknown as typeof schemas.invoices.updateInvoiceLineSchema)
      : schemas.invoices.updateInvoiceLineSchema,
    defaultValues:
      initialData ||
      ({
        name: '',
        startDate: new Date(),
        endDate: (() => {
          const d = new Date()
          d.setDate(d.getDate() + 1)
          return d
        })(),
        quantity: hasSublines ? undefined : 1.0,
        unitPrice: hasSublines ? undefined : 1.0,
        taxRate: 20.0,
        description: '',
      } as unknown as UpdateInvoiceLineSchema),
  })

  useEffect(() => {
    if (initialData) {
      lineItemMethods.reset(initialData as UpdateInvoiceLineSchema)
    }
  }, [])

  const sublinesArray = useFieldArray({
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    control: lineItemMethods.control as any,
    name: 'subLines' as never,
  })

  const handleSubmit = (data: UpdateInvoiceLineSchema) => {
    const itemToSave = {
      ...data,
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      id: (initialData as any)?.lineItemId,
    }
    onSave(itemToSave as LineModalValue)
    onClose()
    lineItemMethods.reset()
  }

  const handleDateRangeChange = (dateRange: { from?: Date; to?: Date } | undefined) => {
    const newStartDate = dateRange?.from || new Date()
    let newEndDate = dateRange?.to || new Date()

    if (newEndDate <= newStartDate) {
      newEndDate = new Date(newStartDate)
      newEndDate.setDate(newEndDate.getDate() + 1)
    }

    lineItemMethods.setValue('startDate', newStartDate)
    lineItemMethods.setValue('endDate', newEndDate)
  }

  const watchQuantity = lineItemMethods.watch('quantity')
  const watchUnitPrice = lineItemMethods.watch('unitPrice')
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const watchSubLines = lineItemMethods.watch('subLines' as any) as
    | Array<{ quantity: number; unitPrice: number }>
    | undefined

  const sublinesTotalMinor =
    watchSubLines?.reduce(
      (acc, sl) =>
        acc +
        Number(
          majorToMinorUnit(Number(sl.quantity ?? 0) * Number(sl.unitPrice ?? 0), currency)
        ),
      0
    ) ?? 0

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className={hasSublines ? 'max-w-2xl' : 'max-w-md'}>
        <Form {...lineItemMethods}>
          <form onSubmit={lineItemMethods.handleSubmit(handleSubmit)}>
            <DialogHeader>
              <DialogTitle>{initialData ? 'Edit Line Item' : 'Add Line Item'}</DialogTitle>
            </DialogHeader>
            <div className="space-y-4 py-4">
              {(isUsageBased || hasSublines) && (
                <div className="flex items-center gap-2 p-3 bg-muted/50 border border-border rounded-md">
                  <Info size={16} className="text-muted-foreground flex-shrink-0" />
                  <span className="text-xs text-foreground">
                    {isUsageBased && !hasSublines && (
                      <>
                        This is a usage-based line item. Quantity will be calculated from metrics on
                        finalization.
                      </>
                    )}
                    {hasSublines && (
                      <>
                        This line item is composed of sublines. Edit each subline&apos;s quantity and
                        unit price; the line total is the sum of subline totals.
                      </>
                    )}
                  </span>
                </div>
              )}

              <GenericFormField
                control={lineItemMethods.control}
                layout="vertical"
                label="Product Name"
                name="name"
                render={({ field }) => (
                  <Input {...field} placeholder="Product name" autoComplete="off" />
                )}
              />

              {!hasSublines && (
                <div>
                  <Label className="font-normal text-xs text-muted-foreground mb-2 block">
                    Date Range
                    {isUsageBased && ' (read-only)'}
                  </Label>
                  <TooltipProvider>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <div>
                          <DatePickerWithRange
                            range={{
                              from: lineItemMethods.watch('startDate'),
                              to: lineItemMethods.watch('endDate'),
                            }}
                            setRange={range => handleDateRangeChange(range)}
                            disabled={isUsageBased}
                          />
                        </div>
                      </TooltipTrigger>
                      {isUsageBased && (
                        <TooltipContent>
                          <p>
                            Date range is determined by the subscription period for usage-based
                            items
                          </p>
                        </TooltipContent>
                      )}
                    </Tooltip>
                  </TooltipProvider>
                </div>
              )}

              {!hasSublines && (
                <div>
                  <Label className="font-normal text-xs text-muted-foreground mb-2 block disabled">
                    Quantity {isUsageBased && ' (read-only)'}
                  </Label>
                  <TooltipProvider>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <div>
                          <GenericFormField
                            control={lineItemMethods.control}
                            layout="vertical"
                            containerClassName="max-w-[250px]"
                            name="quantity"
                            render={({ field }) => (
                              <Input
                                {...field}
                                type="number"
                                step="0.01"
                                min="0.01"
                                value={field.value}
                                onChange={e => field.onChange(Number(e.target.value) || 0.0)}
                                disabled={isUsageBased}
                                autoComplete="off"
                              />
                            )}
                          />
                        </div>
                      </TooltipTrigger>
                      {isUsageBased && (
                        <TooltipContent>
                          <p>Quantity calculated from usage metrics on finalization</p>
                        </TooltipContent>
                      )}
                    </Tooltip>
                  </TooltipProvider>
                </div>
              )}
              {!hasSublines && (
                <div>
                  <Label className="font-normal text-xs text-muted-foreground mb-2 block">
                    Unit Price
                  </Label>
                  <GenericFormField
                    control={lineItemMethods.control}
                    layout="vertical"
                    containerClassName="max-w-[250px]"
                    name="unitPrice"
                    render={({ field }) => (
                      <UncontrolledPriceInput
                        {...field}
                        currency={currency}
                        showCurrency={false}
                        precision={2}
                        value={field.value}
                        onChange={e => field.onChange(Number(e.target.value) || 0.0)}
                        autoComplete="off"
                      />
                    )}
                  />
                </div>
              )}

              {hasSublines && (
                <div className="space-y-2">
                  <Label className="font-normal text-xs text-muted-foreground block">
                    Sublines
                  </Label>
                  <div className="border rounded-md divide-y">
                    <div className="grid grid-cols-[1fr_90px_120px_100px_28px] gap-2 px-2 py-1.5 text-[10px] uppercase tracking-wide text-muted-foreground bg-muted/30">
                      <div>Name</div>
                      <div className="text-right">Qty</div>
                      <div className="text-right">Unit price</div>
                      <div className="text-right">Total</div>
                      <div />
                    </div>
                    {sublinesArray.fields.length === 0 && (
                      <div className="px-2 py-3 text-center text-xs text-muted-foreground">
                        No sublines. Add at least one.
                      </div>
                    )}
                    {sublinesArray.fields.map((field, idx) => {
                      const row = watchSubLines?.[idx]
                      const rowTotalMinor = Number(
                        majorToMinorUnit(
                          Number(row?.quantity ?? 0) * Number(row?.unitPrice ?? 0),
                          currency
                        )
                      )
                      return (
                        <div
                          key={field.id}
                          className="grid grid-cols-[1fr_90px_120px_100px_28px] gap-2 px-2 py-1.5 items-center"
                        >
                          <GenericFormField
                            control={lineItemMethods.control}
                            layout="vertical"
                            // eslint-disable-next-line @typescript-eslint/no-explicit-any
                            name={`subLines.${idx}.name` as any}
                            render={({ field: f }) => (
                              <Input
                                {...f}
                                placeholder="Subline name"
                                className="h-8 text-xs"
                                autoComplete="off"
                              />
                            )}
                          />
                          <GenericFormField
                            control={lineItemMethods.control}
                            layout="vertical"
                            // eslint-disable-next-line @typescript-eslint/no-explicit-any
                            name={`subLines.${idx}.quantity` as any}
                            render={({ field: f }) => (
                              <Input
                                {...f}
                                type="number"
                                step="0.01"
                                min="0"
                                value={f.value}
                                onChange={e => f.onChange(Number(e.target.value) || 0)}
                                className="h-8 text-xs text-right"
                                autoComplete="off"
                              />
                            )}
                          />
                          <GenericFormField
                            control={lineItemMethods.control}
                            layout="vertical"
                            // eslint-disable-next-line @typescript-eslint/no-explicit-any
                            name={`subLines.${idx}.unitPrice` as any}
                            render={({ field: f }) => (
                              <UncontrolledPriceInput
                                {...f}
                                currency={currency}
                                showCurrency={false}
                                precision={2}
                                value={f.value}
                                onChange={e => f.onChange(Number(e.target.value) || 0)}
                                className="h-8 text-xs text-right"
                                autoComplete="off"
                              />
                            )}
                          />
                          <div className="text-xs text-right tabular-nums">
                            {formatCurrency(rowTotalMinor, currency)}
                          </div>
                          <Button
                            type="button"
                            variant="link"
                            size="icon"
                            className="h-6 w-6 p-0 text-destructive hover:text-destructive"
                            onClick={() => sublinesArray.remove(idx)}
                          >
                            <Trash2 size={14} />
                          </Button>
                        </div>
                      )
                    })}
                  </div>
                  <div className="flex items-center justify-between">
                    <Button
                      type="button"
                      variant="link"
                      size="sm"
                      className="h-7 px-0 text-xs"
                      onClick={() =>
                        // eslint-disable-next-line @typescript-eslint/no-explicit-any
                        sublinesArray.append({
                          name: '',
                          quantity: 1,
                          unitPrice: 0,
                          // eslint-disable-next-line @typescript-eslint/no-explicit-any
                        } as any)
                      }
                    >
                      <Plus size={12} className="mr-1" />
                      Add subline
                    </Button>
                    <div className="text-xs">
                      <span className="text-muted-foreground mr-2">Line total</span>
                      <span className="font-medium tabular-nums">
                        {formatCurrency(sublinesTotalMinor, currency)}
                      </span>
                    </div>
                  </div>
                </div>
              )}

              <GenericFormField
                control={lineItemMethods.control}
                layout="vertical"
                label="Tax Rate (%)"
                containerClassName="max-w-[250px]"
                name="taxRate"
                render={({ field }) => (
                  <Input
                    {...field}
                    type="number"
                    step="0.01"
                    min="0"
                    max="100"
                    value={field.value}
                    onChange={e => field.onChange(Number(e.target.value) || 0)}
                    autoComplete="off"
                  />
                )}
              />

              <GenericFormField
                control={lineItemMethods.control}
                layout="vertical"
                label="Description (optional)"
                name="description"
                render={({ field }) => (
                  <Textarea
                    {...field}
                    placeholder="Additional details..."
                    value={field.value || ''}
                    rows={2}
                  />
                )}
              />

              {isUsageBased || hasSublines || !watchUnitPrice ? null : (
                <div className="pt-2 flex justify-between">
                  <div className="text-xs font-medium mb-2">Total (excl. tax)</div>
                  <div className="text-right font-medium">
                    {formatCurrency(
                      Number(watchQuantity) * Number(majorToMinorUnit(watchUnitPrice, currency)),
                      currency
                    )}
                  </div>
                </div>
              )}
            </div>
            <DialogFooter>
              <Button type="button" variant="secondary" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit">{initialData ? 'Save' : 'Add Item'}</Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
