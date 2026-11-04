import { create, type MessageInitShape } from '@bufbuild/protobuf';
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
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useFieldArray } from 'react-hook-form'
import { toast } from 'sonner'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { Slot } from '@/edition'
import {
  BillingInfoFormValues,
  billingInfoSchema,
} from '@/features/customers/components/BillingInfoForm'
import { InvoiceEditBillingInfo } from '@/features/customers/components/InvoiceEditBillingInfo'
import {
  CustomPropertiesFields,
  parseCustomProperties,
  serializeCustomProperties,
} from '@/features/customproperties/CustomPropertiesFields'
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
  SubLineSchema,
  UpdateInvoiceLineSchema,
  UpdateInvoiceLineSchemaRegular,
  UpdateInvoiceLineSchemaWithSublines,
  UpdateInvoiceSchema,
} from '@/lib/schemas/invoices'
import { parseDate } from '@/lib/utils/date'
import { percentToRate, rateToPercent , formatCurrency, minorToMajorUnit } from '@/lib/utils/numbers'
import { resizeSvgContent } from '@/pages/tenants/invoice/utils'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { AddressSchema } from '@/rpc/api/customers/v1/models_pb';
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import {
  getInvoice,
  previewInvoiceUpdate,
  previewNewInvoiceSvg,
  updateInvoice,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import {
  DetailedInvoice,
  InvoiceAnnex,
  SubLineItemSchema,
  UpdateInlineCustomerSchema,
  UpdateInvoiceLineItemSchema,
  UpdateInvoiceRequestSchema,
  UpdateInvoiceRequest_UpdatedAnnexesSchema,
  UpdateInvoiceRequest_UpdatedLineItemsSchema,
} from '@/rpc/api/invoices/v1/models_pb';

import type { Address } from '@/rpc/api/customers/v1/models_pb';

interface InvoiceEditFormProps {
  invoice: DetailedInvoice
  invoiceId: string
  onCancel: () => void
  onSuccess: () => void
}

const mapBillingInfoToAddress = (values: BillingInfoFormValues): Address | undefined => {
  if (
    !values.line1 &&
    !values.line2 &&
    !values.city &&
    !values.country &&
    !values.state &&
    !values.zipCode
  )
    return undefined

  return create(AddressSchema, {
    line1: values.line1 || undefined,
    line2: values.line2 || undefined,
    city: values.city || undefined,
    country: values.country || undefined,
    state: values.state || undefined,
    zipCode: values.zipCode || undefined,
  });
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

const mapSublinesForApi = (subLines: SubLineSchema[]): MessageInitShape<typeof SubLineItemSchema>[] =>
  subLines.map(sl => ({
    id: sl.id ?? '',
    name: sl.name,
    // `total` is recomputed server-side from quantity * unit_price using currency precision.
    total: 0n,
    quantity: sl.quantity.toString(),
    unitPrice: sl.unitPrice.toString(),
    sublineAttributes: { case: undefined },
  }))

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

  /**
   * A correction reissues a fully credited invoice from a detached clone of its lines, so its
   * annexes are a frozen snapshot the operator owns. Every other draft re-derives them from the
   * event stream, and an edit here would be overwritten on the next refresh.
   */
  const isCorrection = !!invoice.parentInvoiceId
  // Lines priced in credits are only settled against the wallet at finalization, so their
  // amounts (and a discount over them) cannot be frozen by hand; other fields stay editable.
  const billingLocked = invoice.lineItems.some(item => item.creditUnit !== undefined)
  const [annexes, setAnnexes] = useState<InvoiceAnnex[]>(invoice.annexes)
  const [annexesEdited, setAnnexesEdited] = useState(false)

  const billingMethods = useZodForm({
    schema: billingInfoSchema,
    defaultValues: {
      name: invoice.customerDetails?.name || '',
      billingEmail: invoice.customerDetails?.email || '',
      line1: invoice.customerDetails?.billingAddress?.line1 || '',
      line2: invoice.customerDetails?.billingAddress?.line2 || '',
      city: invoice.customerDetails?.billingAddress?.city || '',
      state: invoice.customerDetails?.billingAddress?.state || '',
      zipCode: invoice.customerDetails?.billingAddress?.zipCode || '',
      country: invoice.customerDetails?.billingAddress?.country || undefined,
      vatNumber: invoice.customerDetails?.vatNumber || '',
    },
  })

  const convertLineItemsToForm = useCallback((): UpdateInvoiceSchema['lines'] => {
    return invoice.lineItems.map(storedItem => {
      // A tax-inclusive invoice is edited in the figures it was quoted in: the stored line is
      // net, but the operator sees and types tax-included prices, and the backend carves the
      // tax back out on save.
      const quote = storedItem.taxInclusiveQuote
      const item = quote
        ? {
            ...storedItem,
            unitPrice: quote.unitPrice,
            subLineItems: quote.subLineItems,
          }
        : storedItem
      const hasSublines = item.subLineItems && item.subLineItems.length > 0

      if (hasSublines) {
        return {
          lineItemId: item.id,
          name: item.name,
          startDate: parseDate(item.startDate),
          endDate: parseDate(item.endDate),
          taxRate: rateToPercent(item.taxRate),
          description: item.description,
          metricId: item.metricId,
          subLines: item.subLineItems.map(sl => ({
            id: sl.id,
            name: sl.name,
            quantity: parseFloat(sl.quantity || '0'),
            unitPrice: parseFloat(sl.unitPrice || '0'),
          })),
        } as UpdateInvoiceLineSchemaWithSublines
      } else {
        return {
          lineItemId: item.id,
          name: item.name,
          startDate: parseDate(item.startDate),
          endDate: parseDate(item.endDate),
          quantity: parseFloat(item.quantity || '0'),
          unitPrice: parseFloat(item.unitPrice || '0'),
          taxRate: rateToPercent(item.taxRate),
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
      discount: minorToMajorUnit(invoice.quotedDiscount ?? invoice.discount, invoice.currency),
      lines: convertLineItemsToForm(),
      customProperties: parseCustomProperties(invoice.customProperties),
    },
  })

  const { fields, append, remove, update } = useFieldArray({
    control: methods.control,
    name: 'lines',
  })

  // The prefilled due date is the projected value; only persist it when the operator actually
  // changed it, so we don't echo the projection back and defeat the emission-date rebase.
  // Compare the yyyy-MM-dd strings that actually get sent (value-based and timezone-consistent),
  // not the Date objects — parseDate yields UTC midnight while a picker emits local midnight.
  const dueDateOverridden = (d?: Date) => {
    const projected = invoice.dueAt ? mapDatev2(parseDate(invoice.dueAt)) : undefined
    const current = d ? mapDatev2(d) : undefined
    return projected !== current
  }

  const updateInvoiceMutation = useMutation(updateInvoice, {
    onSuccess: async () => {
      toast.success('Invoice updated successfully')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoice,
          input: { id: invoiceId },
          cardinality: 'finite'
        }),
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

  /**
   * Set by "Refresh from customer record" and cleared by any subsequent manual billing edit.
   * A ref rather than state: `callPreview` fires in the same tick as the click, before a state
   * update would be visible to it.
   */
  const resyncFromCustomerRef = useRef(false)

  /**
   * `refreshFromCustomer` makes the backend re-read the customer and drop this invoice's override;
   * the proto ignores the inline fields in that case, so they're omitted rather than sent stale.
   */
  const buildCustomerDetailsPayload = () => {
    if (resyncFromCustomerRef.current) {
      return create(UpdateInlineCustomerSchema, { refreshFromCustomer: true })
    }

    const details = mapCustomerDetailsForApi(billingMethods.getValues())
    return create(UpdateInlineCustomerSchema, {
      refreshFromCustomer: false,
      name: details.name || undefined,
      vatNumber: details.vatNumber || undefined,
      billingAddress: details.billingAddress,
    })
  }

  const buildAnnexesPayload = () =>
    isCorrection && annexesEdited
      ? create(UpdateInvoiceRequest_UpdatedAnnexesSchema, { items: annexes })
      : undefined

  const handleAnnexesChange = (next: InvoiceAnnex[]) => {
    setAnnexes(next)
    setAnnexesEdited(true)
    callPreview(methods.getValues())
  }

  const handleAnnexesRegenerated = (next: InvoiceAnnex[]) => {
    setAnnexes(next)
    setAnnexesEdited(false)
    callPreview(methods.getValues())
  }

  const callPreview = useDebouncedCallback(async (formData: UpdateInvoiceSchema) => {
    try {
      setIsPreviewLoading(true)
      setIsSvgPreviewLoading(true)

      const lineItems: MessageInitShape<typeof UpdateInvoiceLineItemSchema>[] = (formData.lines || []).map(line => {
        const lineWithSublines = line as UpdateInvoiceLineSchemaWithSublines
        const hasSublines = !!lineWithSublines.subLines && lineWithSublines.subLines.length > 0
        const lineWithValues = line as UpdateInvoiceLineSchemaRegular

        return {
          id: line.lineItemId,
          name: line.name,
          startDate: mapDatev2(line.startDate),
          endDate: mapDatev2(line.endDate),
          quantity: hasSublines ? undefined : lineWithValues.quantity?.toString(),
          unitPrice: hasSublines ? undefined : lineWithValues.unitPrice?.toString(),
          taxRate: percentToRate(line.taxRate || 0),
          description: line.description,
          metricId: line.metricId,
          subLineItems: hasSublines ? mapSublinesForApi(lineWithSublines.subLines) : [],
        }
      })

      const updateRequest = create(UpdateInvoiceRequestSchema, {
        id: invoice.id,
        memo: formData.memo || undefined,
        reference: formData.reference || undefined,
        purchaseOrder: formData.purchaseOrder || undefined,
        dueDate:
          dueDateOverridden(formData.dueDate) && formData.dueDate
            ? mapDatev2(formData.dueDate)
            : undefined,
        discount: !billingLocked && formData.discount ? formData.discount.toString() : undefined,
        lineItems:
          !billingLocked && lineItems.length > 0
            ? create(UpdateInvoiceRequest_UpdatedLineItemsSchema, { items: lineItems })
            : undefined,
        annexes: buildAnnexesPayload(),
      })

      updateRequest.customerDetails = buildCustomerDetailsPayload()

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

      // Omitted while re-syncing so the backend renders from the customer record, matching what
      // the save will persist. Otherwise the operator's edited bill-to is what should be drawn.
      const svgCustomerDetails = resyncFromCustomerRef.current
        ? undefined
        : mapCustomerDetailsForApi(billingMethods.getValues())
      const svgResult = await svgPreviewMutation.mutateAsync({
        invoice: {
          customerId: invoice.customerId,
          // invoice.invoiceDate is already a yyyy-MM-dd string; pass it through rather than
          // round-tripping via a UTC-parsed Date, which formats to the previous day in negative
          // UTC offsets.
          invoiceDate: invoice.invoiceDate || mapDatev2(new Date()),
          // This is a pure rendering payload (not an update), so send the due date unconditionally
          // — omitting it would blank the due date in the preview PDF. The override guard belongs
          // only on the two UpdateInvoiceRequest payloads.
          dueDate: formData.dueDate ? mapDatev2(formData.dueDate) : undefined,
          currency: invoice.currency,
          purchaseOrder: formData.purchaseOrder || undefined,
          discount: formData.discount ? formData.discount.toString() : undefined,
          memo: formData.memo || undefined,
          reference: formData.reference || undefined,
          lineItems: (formData.lines || []).map(line => {
            const lineWithSublines = line as UpdateInvoiceLineSchemaWithSublines
            const hasSublines =
              !!lineWithSublines.subLines && lineWithSublines.subLines.length > 0
            const lineWithValues = line as UpdateInvoiceLineSchemaRegular

            return {
              product: line.name,
              startDate: mapDatev2(line.startDate),
              endDate: mapDatev2(line.endDate),
              quantity: hasSublines ? undefined : lineWithValues.quantity?.toString(),
              unitPrice: hasSublines ? undefined : lineWithValues.unitPrice?.toString(),
              taxRate: percentToRate(line.taxRate || 0),
              description: line.description,
              subLineItems: hasSublines ? mapSublinesForApi(lineWithSublines.subLines) : [],
            }
          }),
          customerDetails: svgCustomerDetails,
          annexes,
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
      const updateRequest = create(UpdateInvoiceRequestSchema, {
        id: invoice.id,
        memo: data.memo || undefined,
        reference: data.reference || undefined,
        purchaseOrder: data.purchaseOrder || undefined,
        dueDate:
          dueDateOverridden(data.dueDate) && data.dueDate ? mapDatev2(data.dueDate) : undefined,
        discount: !billingLocked && data.discount ? data.discount.toString() : undefined,
        customProperties: serializeCustomProperties(data.customProperties),
      })

      if (!billingLocked && data.lines && data.lines.length > 0) {
        const lineItems: MessageInitShape<typeof UpdateInvoiceLineItemSchema>[] = data.lines.map(line => {
          const lineWithSublines = line as UpdateInvoiceLineSchemaWithSublines
          const hasSublines = !!lineWithSublines.subLines && lineWithSublines.subLines.length > 0
          const lineWithValues = line as UpdateInvoiceLineSchemaRegular

          return {
            id: line.lineItemId,
            name: line.name,
            startDate: mapDatev2(line.startDate),
            endDate: mapDatev2(line.endDate),
            quantity: hasSublines ? undefined : lineWithValues.quantity?.toString(),
            unitPrice: hasSublines ? undefined : lineWithValues.unitPrice?.toString(),
            taxRate: percentToRate(line.taxRate || 0),
            description: line.description,
            metricId: line.metricId,
            subLineItems: hasSublines ? mapSublinesForApi(lineWithSublines.subLines) : [],
          }
        })

        updateRequest.lineItems = create(UpdateInvoiceRequest_UpdatedLineItemsSchema, {
          items: lineItems,
        })
      }

      updateRequest.customerDetails = buildCustomerDetailsPayload()
      updateRequest.annexes = buildAnnexesPayload()

      await updateInvoiceMutation.mutateAsync(updateRequest)
      resyncFromCustomerRef.current = false
    } catch (error) {
      console.error('Failed to update invoice:', error)
    }
  }

  const handleAddLine = (item: UpdateInvoiceLineSchema | UpdateInvoiceLineSchemaWithSublines) => {
    append(item as UpdateInvoiceLineSchema)
    setIsLineItemModalOpen(false)
    callPreview(methods.getValues())
  }

  const handleEditLine = (index: number) => {
    setEditingLineIndex(index)
    setIsLineItemModalOpen(true)
  }

  const handleSaveEditedLine = (
    item: UpdateInvoiceLineSchema | UpdateInvoiceLineSchemaWithSublines
  ) => {
    if (editingLineIndex !== null) {
      update(editingLineIndex, item as UpdateInvoiceLineSchema)
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
          state: result.customer.billingAddress?.state || '',
          zipCode: result.customer.billingAddress?.zipCode || '',
          country: result.customer.billingAddress?.country || undefined,
          vatNumber: result.customer.vatNumber || '',
        })

        setIsEditingBilling(true)

        // Tells the backend to drop this invoice's override on the next preview/save, so the
        // fields the form can't show (email, language) re-sync too.
        resyncFromCustomerRef.current = true

        toast.success('Customer details reloaded from customer record')
        callPreview(methods.getValues())
      }
    } catch (error) {
      console.error('Failed to load customer details:', error)
      toast.error('Failed to load customer details')
    }
  }

  const handleBillingInfoChange = async (_values: BillingInfoFormValues) => {
    // A manual edit outranks a pending re-sync — but only a real one. The shared form fires
    // onBlur on every field exit, and handleRefreshCustomer resets the form (clearing dirty),
    // so without this check merely tabbing through would downgrade the re-sync to an inline
    // edit and leave email / preferred locales diverged on an already-overridden draft.
    if (billingMethods.formState.isDirty) {
      resyncFromCustomerRef.current = false
    }
    callPreview(methods.getValues())
  }

  const totals = useMemo(() => {
    if (!previewData) return null

    return {
      subtotal: previewData.subtotal,
      discount: previewData.discount,
      promotionalCredits: previewData.promotionalCredits,
      tax: previewData.taxAmount,
      total: previewData.total,
    }
  }, [previewData])

  useEffect(() => {
    callPreview(methods.getValues())
  }, [])

  return (
    <Flex className="flex-col lg:flex-row lg:h-full">
      {/* Left Panel - Edit Form */}
      <Flex
        direction="column"
        className="w-full lg:w-1/3 border-b lg:border-b-0 lg:border-r border-border"
      >
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
                {invoice.taxInclusive && (
                  <div className="text-xs text-muted-foreground">
                    Prices and the discount on this invoice include tax. The tax is carved out of
                    them, so the total is what you enter.
                  </div>
                )}
                {billingLocked && (
                  <div className="text-xs text-muted-foreground">
                    This draft bills usage priced in credits. What the credit balance covers is
                    only settled when the invoice is finalized, so its lines and discount cannot
                    be edited by hand. Change the subscription instead, or issue a credit note
                    once it is finalized.
                  </div>
                )}

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
                        readOnly={billingLocked}
                      />
                    )
                  })}

                  {fields.length === 0 && (
                    <div className="text-center py-4 text-sm text-muted-foreground">
                      No line items. Add at least one line item.
                    </div>
                  )}
                </div>

                {!billingLocked && (
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
                )}
              </div>

              {billingLocked ? null : showDiscount || invoice.discount > 0 ? (
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

              <Slot
                name="invoiceEdit.annexes"
                invoiceId={invoice.id}
                annexes={annexes}
                isCorrection={isCorrection}
                onChange={handleAnnexesChange}
                onRegenerated={handleAnnexesRegenerated}
              />

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

                <CustomPropertiesFields
                  control={methods.control}
                  entityType={CustomPropertyEntityType.INVOICE}
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
                      {totals.promotionalCredits > 0 && (
                        <div className="flex justify-between">
                          <span className="text-muted-foreground">Promotional credit</span>
                          <span>
                            -{formatCurrency(totals.promotionalCredits, invoice.currency)}
                          </span>
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
      <div className="w-full lg:w-2/3 flex flex-col">
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
        taxInclusive={invoice.taxInclusive}
        initialData={
          editingLineIndex !== null
            ? (fields[editingLineIndex] as UpdateInvoiceLineSchema)
            : undefined
        }
        isUsageBased={
          editingLineIndex !== null && !isCorrection ? !!fields[editingLineIndex].metricId : false
        }
        hasSublines={
          editingLineIndex !== null
            ? !!(fields[editingLineIndex] as UpdateInvoiceLineSchemaWithSublines).subLines?.length
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
