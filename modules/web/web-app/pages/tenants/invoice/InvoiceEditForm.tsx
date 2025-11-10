import { PlainMessage } from '@bufbuild/protobuf'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  DatePicker,
  Flex,
  Form,
  GenericFormField,
  Input,
  Separator,
  Textarea,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useFieldArray } from 'react-hook-form'
import { toast } from 'sonner'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import {
  BillingInfoFormValues,
  billingInfoSchema,
} from '@/features/customers/components/BillingInfoForm'
import { InvoiceEditBillingInfo } from '@/features/customers/components/InvoiceEditBillingInfo'
import { DeleteConfirmDialog } from '@/features/invoices/edit/DeleteConfirmDialog'
import { EditInvoicePreview } from '@/features/invoices/edit/EditInvoicePreview'
import { LineItemDisplay } from '@/features/invoices/edit/LineItemDisplay'
import { LineItemModal } from '@/features/invoices/edit/LineItemModal'
import { useDebouncedCallback } from '@/hooks/useDebounce'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { mapDatev2 } from '@/lib/mapping'
import { schemas } from '@/lib/schemas'
import {
  OriginalLineItem,
  UpdateInvoiceLineSchema,
  UpdateInvoiceLineSchemaRegular,
  UpdateInvoiceSchema,
} from '@/lib/schemas/invoices'
import { parseDate } from '@/lib/utils/date'
import { resizeSvgContent } from '@/pages/tenants/invoice/utils'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { Address } from '@/rpc/api/customers/v1/models_pb'
import {
  getInvoice,
  previewInvoiceUpdate,
  previewNewInvoiceSvg,
  updateInvoice,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import {
  DetailedInvoice,
  SubLineItem,
  UpdateInlineCustomer,
  UpdateInvoiceLineItem,
  UpdateInvoiceRequest,
  UpdateInvoiceRequest_UpdatedLineItems,
} from '@/rpc/api/invoices/v1/models_pb'
import { formatCurrency, minorToMajorUnit } from '@/utils/numbers'

interface InvoiceEditFormProps {
  invoice: DetailedInvoice
  invoiceId: string
  onCancel: () => void
  onSuccess: () => void
}

const mapBillingInfoToAddress = (values: BillingInfoFormValues): Address | undefined => {
  if (!values.line1 && !values.city && !values.country) return undefined

  return new Address({
    line1: values.line1 || undefined,
    line2: values.line2 || undefined,
    city: values.city || undefined,
    country: values.country || undefined,
    state: undefined,
    zipCode: values.zipCode || undefined,
  })
}

const mapCustomerDetailsForApi = (
  billingValues: BillingInfoFormValues
): { name: string; email?: string; vatNumber?: string; billingAddress?: Address } => {
  return {
    name: billingValues.name || '',
    email: billingValues.billingEmail || undefined,
    vatNumber: billingValues.vatNumber || undefined,
    billingAddress: mapBillingInfoToAddress(billingValues),
  }
}

export const InvoiceEditForm: React.FC<InvoiceEditFormProps> = ({
  invoice,
  invoiceId,
  onCancel,
  onSuccess,
}) => {
  const queryClient = useQueryClient()
  const [isLineItemModalOpen, setIsLineItemModalOpen] = useState(false)
  const [editingLineIndex, setEditingLineIndex] = useState<number | null>(null)
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false)
  const [itemToDelete, setItemToDelete] = useState<{ index: number; name: string } | null>(null)
  const [previewData, setPreviewData] = useState<DetailedInvoice | null>(null)
  const [isPreviewLoading, setIsPreviewLoading] = useState(false)
  const [previewSvgs, setPreviewSvgs] = useState<string[]>([])
  const [isSvgPreviewLoading, setIsSvgPreviewLoading] = useState(false)
  const [isEditingBilling, setIsEditingBilling] = useState(false)
  const [showDiscount, setShowDiscount] = useState(false)

  const [originalLineItems, setOriginalLineItems] = useState<Map<string, OriginalLineItem>>(
    () => new Map(invoice.lineItems.map(item => [item.id, item]))
  )

  const billingMethods = useZodForm({
    schema: billingInfoSchema,
    defaultValues: {
      name: invoice.customerDetails?.name || '',
      billingEmail: invoice.customerDetails?.email || '',
      line1: invoice.customerDetails?.billingAddress?.line1 || '',
      line2: invoice.customerDetails?.billingAddress?.line2 || '',
      city: invoice.customerDetails?.billingAddress?.city || '',
      zipCode: invoice.customerDetails?.billingAddress?.zipCode || '',
      country: invoice.customerDetails?.billingAddress?.country || undefined,
      vatNumber: invoice.customerDetails?.vatNumber || '',
    },
  })

  const convertLineItemsToForm = useCallback((): UpdateInvoiceSchema['lines'] => {
    return invoice.lineItems.map(item => {
      const hasSublines = item.subLineItems && item.subLineItems.length > 0

      if (hasSublines) {
        return {
          lineItemId: item.id,
          name: item.name,
          startDate: parseDate(item.startDate),
          endDate: parseDate(item.endDate),
          taxRate: parseFloat(item.taxRate) * 100,
          description: item.description,
          metricId: item.metricId,
        }
      } else {
        return {
          lineItemId: item.id,
          name: item.name,
          startDate: parseDate(item.startDate),
          endDate: parseDate(item.endDate),
          quantity: parseFloat(item.quantity || '0'),
          unitPrice: parseFloat(item.unitPrice || '0'),
          taxRate: parseFloat(item.taxRate) * 100,
          description: item.description,
          metricId: item.metricId,
        }
      }
    })
  }, [invoice])

  const methods = useZodForm({
    schema: schemas.invoices.updateInvoiceSchema,
    defaultValues: {
      id: invoice.id,
      memo: invoice.memo || '',
      reference: invoice.reference || '',
      purchaseOrder: invoice.purchaseOrder || '',
      dueDate: invoice.dueAt ? parseDate(invoice.dueAt) : undefined,
      discount: minorToMajorUnit(invoice.discount, invoice.currency),
      lines: convertLineItemsToForm(),
    },
  })

  const { fields, append, remove, update } = useFieldArray({
    control: methods.control,
    name: 'lines',
  })

  const updateInvoiceMutation = useMutation(updateInvoice, {
    onSuccess: async () => {
      toast.success('Invoice updated successfully')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getInvoice, { id: invoiceId }),
      })
      onSuccess()
    },
    onError: (error: Error) => {
      toast.error(`Failed to update invoice: ${error.message}`)
    },
  })

  const customerQuery = useQuery(
    getCustomerById,
    {
      id: invoice.customerId,
    },
    {
      enabled: false,
    }
  )

  const previewMutation = useMutation(previewInvoiceUpdate)
  const svgPreviewMutation = useMutation(previewNewInvoiceSvg)

  const callPreview = useDebouncedCallback(async (formData: UpdateInvoiceSchema) => {
    try {
      setIsPreviewLoading(true)
      setIsSvgPreviewLoading(true)

      const lineItems: PlainMessage<UpdateInvoiceLineItem>[] = (formData.lines || []).map(line => {
        const originalItem = line.lineItemId ? originalLineItems.get(line.lineItemId) : undefined
        const hasSublines = originalItem?.subLineItems && originalItem.subLineItems.length > 0
        const lineWithValues = line as UpdateInvoiceLineSchemaRegular

        return {
          id: line.lineItemId,
          name: line.name,
          startDate: mapDatev2(line.startDate),
          endDate: mapDatev2(line.endDate),
          quantity: hasSublines ? originalItem.quantity : lineWithValues.quantity?.toString(),
          unitPrice: hasSublines ? originalItem.unitPrice : lineWithValues.unitPrice?.toString(),
          taxRate: ((line.taxRate || 0) / 100).toString(),
          description: line.description,
          metricId: line.metricId,
          subLineItems: hasSublines
            ? ((originalItem.subLineItems ?? []) as PlainMessage<SubLineItem>[])
            : [],
        }
      })

      const updateRequest = new UpdateInvoiceRequest({
        id: invoice.id,
        memo: formData.memo || undefined,
        reference: formData.reference || undefined,
        purchaseOrder: formData.purchaseOrder || undefined,
        dueDate: formData.dueDate ? mapDatev2(formData.dueDate) : undefined,
        discount: formData.discount ? formData.discount.toString() : undefined,
        lineItems:
          lineItems.length > 0
            ? new UpdateInvoiceRequest_UpdatedLineItems({ items: lineItems })
            : undefined,
      })

      const customerDetails = mapCustomerDetailsForApi(billingMethods.getValues())
      updateRequest.customerDetails = new UpdateInlineCustomer({
        refreshFromCustomer: false,
        name: customerDetails.name || undefined,
        vatNumber: customerDetails.vatNumber || undefined,
        billingAddress: customerDetails.billingAddress,
      })

      const result = await previewMutation.mutateAsync({ updateRequest })
      if (result.preview) {
        setPreviewData(result.preview)
        if (result.preview?.lineItems) {
          setOriginalLineItems(prevMap => {
            const newMap = new Map(prevMap)
            result.preview!.lineItems.forEach(item => {
              newMap.set(item.id, item)
            })
            return newMap
          })
        }
      }

      const svgCustomerDetails = mapCustomerDetailsForApi(billingMethods.getValues())
      const svgResult = await svgPreviewMutation.mutateAsync({
        invoice: {
          customerId: invoice.customerId,
          invoiceDate: mapDatev2(invoice.invoiceDate ? parseDate(invoice.invoiceDate) : new Date()),
          dueDate: formData.dueDate ? mapDatev2(formData.dueDate) : undefined,
          currency: invoice.currency,
          purchaseOrder: formData.purchaseOrder || undefined,
          discount: formData.discount ? formData.discount.toString() : undefined,
          memo: formData.memo || undefined,
          reference: formData.reference || undefined,
          lineItems: (formData.lines || []).map(line => {
            const originalItem = line.lineItemId
              ? originalLineItems.get(line.lineItemId)
              : undefined
            const hasSublines = originalItem?.subLineItems && originalItem.subLineItems.length > 0
            const lineWithValues = line as UpdateInvoiceLineSchemaRegular

            return {
              product: line.name,
              startDate: mapDatev2(line.startDate),
              endDate: mapDatev2(line.endDate),
              quantity: hasSublines ? undefined : lineWithValues.quantity?.toString(),
              unitPrice: hasSublines ? undefined : lineWithValues.unitPrice?.toString(),
              taxRate: ((line.taxRate || 0) / 100).toString(),
              description: line.description,
              subLineItems: hasSublines
                ? ((originalItem.subLineItems ?? []) as PlainMessage<SubLineItem>[])
                : [],
            }
          }),
          customerDetails: svgCustomerDetails,
        },
      })

      if (svgResult.svgs) {
        const svgContents = svgResult.svgs.map(svg => {
          const scaledHtml = svg ? resizeSvgContent(svg, 1) : ''
          const parser = new DOMParser()
          const doc = parser.parseFromString(scaledHtml, 'text/html')
          const svgElement = doc.querySelector('svg')
          return svgElement?.outerHTML || ''
        })
        setPreviewSvgs(svgContents)
      }
    } catch (err) {
      console.error('Preview failed:', err)
    } finally {
      setIsPreviewLoading(false)
      setIsSvgPreviewLoading(false)
    }
  }, 300)

  const onSubmit = async (data: UpdateInvoiceSchema) => {
    try {
      const updateRequest = new UpdateInvoiceRequest({
        id: invoice.id,
        memo: data.memo || undefined,
        reference: data.reference || undefined,
        purchaseOrder: data.purchaseOrder || undefined,
        dueDate: data.dueDate ? mapDatev2(data.dueDate) : undefined,
        discount: data.discount ? data.discount.toString() : undefined,
      })

      if (data.lines && data.lines.length > 0) {
        const lineItems: PlainMessage<UpdateInvoiceLineItem>[] = data.lines.map(line => {
          const originalItem = line.lineItemId ? originalLineItems.get(line.lineItemId) : undefined
          const hasSublines = originalItem?.subLineItems && originalItem.subLineItems.length > 0
          const lineWithValues = line as UpdateInvoiceLineSchemaRegular

          return {
            id: line.lineItemId,
            name: line.name,
            startDate: mapDatev2(line.startDate),
            endDate: mapDatev2(line.endDate),
            quantity: hasSublines ? originalItem.quantity : lineWithValues.quantity?.toString(),
            unitPrice: hasSublines ? originalItem.unitPrice : lineWithValues.unitPrice?.toString(),
            taxRate: ((line.taxRate || 0) / 100).toString(),
            description: line.description,
            metricId: line.metricId,
            subLineItems: hasSublines
              ? ((originalItem.subLineItems ?? []) as PlainMessage<SubLineItem>[])
              : [],
          }
        })

        updateRequest.lineItems = new UpdateInvoiceRequest_UpdatedLineItems({
          items: lineItems,
        })
      }

      const submitCustomerDetails = mapCustomerDetailsForApi(billingMethods.getValues())
      updateRequest.customerDetails = new UpdateInlineCustomer({
        refreshFromCustomer: false,
        name: submitCustomerDetails.name || undefined,
        vatNumber: submitCustomerDetails.vatNumber || undefined,
        billingAddress: submitCustomerDetails.billingAddress,
      })

      await updateInvoiceMutation.mutateAsync(updateRequest)
    } catch (error) {
      console.error('Failed to update invoice:', error)
    }
  }

  const handleAddLine = (item: UpdateInvoiceLineSchema) => {
    append(item)
    setIsLineItemModalOpen(false)
    callPreview(methods.getValues())
  }

  const handleEditLine = (index: number) => {
    setEditingLineIndex(index)
    setIsLineItemModalOpen(true)
  }

  const handleSaveEditedLine = (item: UpdateInvoiceLineSchema) => {
    if (editingLineIndex !== null) {
      update(editingLineIndex, item)
      setEditingLineIndex(null)
    }
    setIsLineItemModalOpen(false)
    callPreview(methods.getValues())
  }

  const handleRemoveLine = (index: number) => {
    setItemToDelete({ index, name: fields[index].name })
    setDeleteConfirmOpen(true)
  }

  const confirmDelete = () => {
    if (itemToDelete !== null) {
      remove(itemToDelete.index)
      setItemToDelete(null)
      setTimeout(() => callPreview(methods.getValues()), 100)
    }
    setDeleteConfirmOpen(false)
  }

  const handleRefreshCustomer = async () => {
    try {
      const result = (await customerQuery.refetch()).data

      if (result?.customer) {
        billingMethods.reset({
          name: result.customer.name || '',
          billingEmail: result.customer.billingEmail || '',
          line1: result.customer.billingAddress?.line1 || '',
          line2: result.customer.billingAddress?.line2 || '',
          city: result.customer.billingAddress?.city || '',
          zipCode: result.customer.billingAddress?.zipCode || '',
          country: result.customer.billingAddress?.country || undefined,
          vatNumber: result.customer.vatNumber || '',
        })

        setIsEditingBilling(true)

        toast.success('Customer details reloaded from customer record')
        callPreview(methods.getValues())
      }
    } catch (error) {
      console.error('Failed to load customer details:', error)
      toast.error('Failed to load customer details')
    }
  }

  const handleBillingInfoChange = async (_values: BillingInfoFormValues) => {
    callPreview(methods.getValues())
  }

  const totals = useMemo(() => {
    if (!previewData) return null

    return {
      subtotal: previewData.subtotal,
      discount: previewData.discount,
      tax: previewData.taxAmount,
      total: previewData.total,
    }
  }, [previewData])

  useEffect(() => {
    callPreview(methods.getValues())
  }, [])

  return (
    <Flex className="h-full">
      {/* Left Panel - Edit Form */}
      <Flex direction="column" className="w-1/3 border-r border-border">
        <div className="flex-1 overflow-auto p-6">
          <Form {...methods}>
            <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-6">
              <div>
                <h3 className="text-lg font-medium mb-4">Edit Invoice</h3>
              </div>

              {/* Customer Details Section */}
              <div className="space-y-4">
                <InvoiceEditBillingInfo
                  customerDetails={invoice.customerDetails!}
                  isEditing={isEditingBilling}
                  setIsEditing={setIsEditingBilling}
                  methods={billingMethods}
                  onSubmit={handleBillingInfoChange}
                  onRefreshFromCustomer={handleRefreshCustomer}
                />
              </div>

              <Separator />

              {/* Line Items Section */}
              <div className="space-y-4">
                <div className="text-[15px] font-medium">Line Items</div>

                {methods.formState.errors.lines && (
                  <div className="text-[0.8rem] font-medium text-destructive">
                    {methods.formState.errors.lines.message}
                  </div>
                )}

                <div className="space-y-2 border rounded-md p-3">
                  {fields.map((field, index) => {
                    const originalItem = field.lineItemId
                      ? originalLineItems.get(field.lineItemId)
                      : undefined

                    return (
                      <LineItemDisplay
                        key={field.lineItemId}
                        item={field}
                        index={index}
                        currency={invoice.currency}
                        onRemove={handleRemoveLine}
                        onEdit={handleEditLine}
                        isUsageBased={!!field.metricId}
                        originalItem={originalItem}
                      />
                    )
                  })}

                  {fields.length === 0 && (
                    <div className="text-center py-4 text-sm text-muted-foreground">
                      No line items. Add at least one line item.
                    </div>
                  )}
                </div>

                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setEditingLineIndex(null)
                    setIsLineItemModalOpen(true)
                  }}
                  className="w-full"
                >
                  + Add Line Item
                </Button>
              </div>

              {showDiscount || invoice.discount > 0 ? (
                <div className="space-y-2 w-full flex justify-between items-center">
                  <GenericFormField
                    control={methods.control}
                    layout="horizontal"
                    label="Discount"
                    containerClassName="w-full"
                    labelClassName="col-span-8"
                    name="discount"
                    render={({ field }) => (
                      <UncontrolledPriceInput
                        {...field}
                        currency={invoice.currency}
                        showCurrency={false}
                        className="col-span-4"
                        precision={2}
                        onChange={e => field.onChange(Number(e.target.value) || 0)}
                        onBlur={() => callPreview(methods.getValues())}
                        autoComplete="off"
                      />
                    )}
                  />
                </div>
              ) : (
                <Button
                  type="button"
                  variant="link"
                  className="text-xs px-0"
                  onClick={() => setShowDiscount(true)}
                >
                  + Add Discount
                </Button>
              )}

              <Separator />

              {/* Metadata Section */}
              <div className="space-y-4">
                <div className="text-[15px] font-medium">Metadata</div>

                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="Due Date"
                  name="dueDate"
                  render={({ field }) => (
                    <DatePicker
                      mode="single"
                      captionLayout="dropdown"
                      date={field.value}
                      onSelect={date => {
                        field.onChange(date)
                        callPreview(methods.getValues())
                      }}
                    />
                  )}
                />

                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="Reference"
                  name="reference"
                  render={({ field }) => (
                    <Input
                      {...field}
                      placeholder="Reference number"
                      autoComplete="off"
                      onBlur={() => callPreview(methods.getValues())}
                    />
                  )}
                />

                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="Purchase Order"
                  name="purchaseOrder"
                  render={({ field }) => (
                    <Input
                      {...field}
                      placeholder="PO number"
                      autoComplete="off"
                      onBlur={() => callPreview(methods.getValues())}
                    />
                  )}
                />

                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="Memo"
                  name="memo"
                  render={({ field }) => (
                    <Textarea
                      {...field}
                      placeholder="Internal notes..."
                      rows={3}
                      onBlur={() => callPreview(methods.getValues())}
                    />
                  )}
                />
              </div>

              <Separator />

              {/* Totals Preview */}
              {(totals || isPreviewLoading) && (
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    {isPreviewLoading && (
                      <span className="text-[11px] text-muted-foreground">Calculating...</span>
                    )}
                  </div>
                  {totals ? (
                    <div className="space-y-1 text-sm">
                      <div className="flex justify-between">
                        <span className="text-muted-foreground">Subtotal</span>
                        <span>{formatCurrency(totals.subtotal, invoice.currency)}</span>
                      </div>
                      {totals.discount > 0 && (
                        <div className="flex justify-between">
                          <span className="text-muted-foreground">Discount</span>
                          <span>-{formatCurrency(totals.discount, invoice.currency)}</span>
                        </div>
                      )}
                      <div className="flex justify-between">
                        <span className="text-muted-foreground">Tax</span>
                        <span>{formatCurrency(totals.tax, invoice.currency)}</span>
                      </div>
                      <div className="flex justify-between pt-2 border-t font-semibold">
                        <span>Total</span>
                        <span>{formatCurrency(totals.total, invoice.currency)}</span>
                      </div>
                    </div>
                  ) : (
                    <div className="text-sm text-muted-foreground">Loading totals...</div>
                  )}
                </div>
              )}

              {/* Action Buttons */}
              <div className="flex gap-2 pt-4">
                <Button type="button" variant="secondary" onClick={onCancel} className="flex-1">
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="primary"
                  disabled={updateInvoiceMutation.isPending}
                  className="flex-1"
                >
                  {updateInvoiceMutation.isPending ? 'Saving...' : 'Save Changes'}
                </Button>
              </div>
            </form>
          </Form>
        </div>
      </Flex>

      {/* Right Panel - Invoice Preview */}
      <div className="w-2/3 flex flex-col">
        <div className="flex-1 overflow-auto p-6">
          <EditInvoicePreview previewData={previewSvgs} isLoading={isSvgPreviewLoading} />
        </div>
      </div>

      {/* Line Item Modal */}
      <LineItemModal
        key={editingLineIndex !== null ? `edit-${editingLineIndex}` : 'new'}
        isOpen={isLineItemModalOpen}
        onClose={() => {
          setIsLineItemModalOpen(false)
          setEditingLineIndex(null)
        }}
        onSave={editingLineIndex !== null ? handleSaveEditedLine : handleAddLine}
        currency={invoice.currency}
        initialData={
          editingLineIndex !== null
            ? (fields[editingLineIndex] as UpdateInvoiceLineSchema)
            : undefined
        }
        isUsageBased={editingLineIndex !== null ? !!fields[editingLineIndex].metricId : false}
        hasSublines={
          editingLineIndex !== null && fields[editingLineIndex].lineItemId
            ? !!originalLineItems.get(fields[editingLineIndex].lineItemId!)?.subLineItems?.length
            : false
        }
      />

      {/* Delete Confirmation Dialog */}
      <DeleteConfirmDialog
        open={deleteConfirmOpen}
        onOpenChange={setDeleteConfirmOpen}
        onConfirm={confirmDelete}
        itemName={itemToDelete?.name || ''}
      />
    </Flex>
  )
}
