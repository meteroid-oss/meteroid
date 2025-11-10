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
import { Info } from 'lucide-react'
import { useEffect } from 'react'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { DatePickerWithRange } from '@/features/dashboard/DateRangePicker'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { UpdateInvoiceLineSchema } from '@/lib/schemas/invoices'
import { formatCurrency, majorToMinorUnit } from '@/utils/numbers'

interface LineItemModalProps {
  isOpen: boolean
  onClose: () => void
  onSave: (item: UpdateInvoiceLineSchema) => void
  currency: string
  initialData?: UpdateInvoiceLineSchema
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
          .updateInvoiceLineWithSublinesSchema as typeof schemas.invoices.updateInvoiceLineSchema)
      : schemas.invoices.updateInvoiceLineSchema,
    defaultValues: initialData || {
      product: '',
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
    },
  })

  useEffect(() => {
    if (initialData) {
      lineItemMethods.reset(initialData)
    }
  }, [])

  const handleSubmit = (data: UpdateInvoiceLineSchema) => {
    const itemToSave = {
      ...data,
      id: initialData?.lineItemId,
    }
    onSave(itemToSave)
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

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
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
                    {hasSublines && !isUsageBased && (
                      <>This line item has sublines. Quantity and unit price cannot be edited.</>
                    )}
                    {hasSublines && isUsageBased && (
                      <>
                        This is a usage-based line item with sublines. Quantity will be calculated
                        from metrics, and unit price is derived from sublines.
                      </>
                    )}
                  </span>
                </div>
              )}

              <GenericFormField
                control={lineItemMethods.control}
                layout="vertical"
                label="Product Name"
                name="product"
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
