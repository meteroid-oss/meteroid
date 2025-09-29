import { useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  DatePicker,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Flex,
  Form,
  GenericFormField,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@ui/components'
import { Edit, XIcon } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useFieldArray } from 'react-hook-form'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import PageHeading from '@/components/PageHeading/PageHeading'
import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { DatePickerWithRange } from '@/features/dashboard/DateRangePicker'
import { useBasePath } from '@/hooks/useBasePath'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { mapDatev2 } from '@/lib/mapping'
import { schemas } from '@/lib/schemas'
import { InvoiceLineSchema } from '@/lib/schemas/invoices'
import { resizeSvgContent } from '@/pages/tenants/invoice'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import {
  createInvoice,
  listInvoices,
  previewNewInvoiceSvg,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { getInvoicingEntity } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const formatCurrency = (amount: number, currency: string) => {
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: currency || 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(amount)
}

const AddLineItemModal = ({
  isOpen,
  onClose,
  onAdd,
  currency,
}: {
  isOpen: boolean
  onClose: () => void
  onAdd: (item: InvoiceLineSchema) => void
  currency: string
}) => {
  const [formData, setFormData] = useState<InvoiceLineSchema>(() => {
    const startDate = new Date()
    const endDate = new Date(startDate)
    endDate.setDate(endDate.getDate() + 1)
    return {
      product: '',
      startDate,
      endDate,
      quantity: 1.0,
      unitPrice: 1.0,
      taxRate: 20.0,
    }
  })

  const handleSubmit = () => {
    if (formData.product.trim()) {
      onAdd(formData)
      onClose()
      // Reset form for next use
      const startDate = new Date()
      const endDate = new Date(startDate)
      endDate.setDate(endDate.getDate() + 1)
      setFormData({
        product: '',
        startDate,
        endDate,
        quantity: 1.0,
        unitPrice: 1.0,
        taxRate: 20.0,
      })
    }
  }

  const handleDateRangeChange = (dateRange: { from?: Date; to?: Date } | undefined) => {
    const newStartDate = dateRange?.from || new Date()
    let newEndDate = dateRange?.to || new Date()

    if (newEndDate <= newStartDate) {
      newEndDate = new Date(newStartDate)
      newEndDate.setDate(newEndDate.getDate() + 1)
    }

    setFormData(prev => ({
      ...prev,
      startDate: newStartDate,
      endDate: newEndDate,
    }))
  }

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Add Line Item</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div>
            <label className="text-sm font-medium mb-2 block">Product</label>
            <Input
              placeholder="Product name"
              value={formData.product}
              onChange={e => setFormData(prev => ({ ...prev, product: e.target.value }))}
              autoComplete="off"
            />
          </div>
          <div>
            <label className="text-sm font-medium mb-2 block">Date Range</label>
            <DatePickerWithRange
              range={{ from: formData.startDate, to: formData.endDate }}
              setRange={range => handleDateRangeChange(range)}
            />
          </div>
          <div>
            <label className="text-sm font-medium mb-2 block">Quantity</label>
            <Input
              type="number"
              step="0.01"
              min="0.01"
              value={formData.quantity}
              onChange={e =>
                setFormData(prev => ({ ...prev, quantity: Number(e.target.value) || 0.0 }))
              }
              autoComplete="off"
            />
          </div>
          <div>
            <label className="text-sm font-medium mb-2 block">Unit Price</label>
            <UncontrolledPriceInput
              currency={currency}
              showCurrency={false}
              precision={2}
              value={formData.unitPrice}
              onChange={e =>
                setFormData(prev => ({ ...prev, unitPrice: Number(e.target.value) || 0.0 }))
              }
              autoComplete="off"
            />
          </div>
          <div>
            <label className="text-sm font-medium mb-2 block">Tax Rate (%)</label>
            <Input
              type="number"
              step="0.01"
              min="0"
              max="100"
              value={formData.taxRate}
              onChange={e =>
                setFormData(prev => ({ ...prev, taxRate: Number(e.target.value) || 0 }))
              }
              autoComplete="off"
            />
          </div>
          <div className="pt-2">
            <div className="text-sm font-medium mb-2">Total (excl. tax)</div>
            <div className="text-right font-medium">
              {formatCurrency(Number(formData.quantity) * Number(formData.unitPrice), currency)}
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={!formData.product.trim()}>
            Add Item
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

const EditLineItemModal = ({
  isOpen,
  onClose,
  onSave,
  currency,
  initialData,
}: {
  isOpen: boolean
  onClose: () => void
  onSave: (item: InvoiceLineSchema) => void
  currency: string
  initialData: InvoiceLineSchema
}) => {
  const [formData, setFormData] = useState<InvoiceLineSchema>(initialData)

  useEffect(() => {
    setFormData(initialData)
  }, [initialData])

  const handleSubmit = () => {
    if (formData.product.trim()) {
      onSave(formData)
      onClose()
    }
  }

  const handleDateRangeChange = (dateRange: { from?: Date; to?: Date } | undefined) => {
    const newStartDate = dateRange?.from || new Date()
    let newEndDate = dateRange?.to || new Date()

    if (newEndDate <= newStartDate) {
      newEndDate = new Date(newStartDate)
      newEndDate.setDate(newEndDate.getDate() + 1)
    }

    setFormData(prev => ({
      ...prev,
      startDate: newStartDate,
      endDate: newEndDate,
    }))
  }

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Edit Line Item</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div>
            <label className="text-sm font-medium mb-2 block">Product</label>
            <Input
              placeholder="Product name"
              value={formData.product}
              onChange={e => setFormData(prev => ({ ...prev, product: e.target.value }))}
              autoComplete="off"
            />
          </div>
          <div>
            <label className="text-sm font-medium mb-2 block">Date Range</label>
            <DatePickerWithRange
              range={{ from: formData.startDate, to: formData.endDate }}
              setRange={range => handleDateRangeChange(range)}
            />
          </div>
          <div>
            <label className="text-sm font-medium mb-2 block">Quantity</label>
            <Input
              type="number"
              step="0.01"
              min="0.01"
              value={formData.quantity}
              onChange={e =>
                setFormData(prev => ({ ...prev, quantity: Number(e.target.value) || 0.0 }))
              }
              autoComplete="off"
            />
          </div>
          <div>
            <label className="text-sm font-medium mb-2 block">Unit Price</label>
            <UncontrolledPriceInput
              currency={currency}
              showCurrency={false}
              precision={2}
              value={formData.unitPrice}
              onChange={e =>
                setFormData(prev => ({ ...prev, unitPrice: Number(e.target.value) || 0.0 }))
              }
              autoComplete="off"
            />
          </div>
          <div>
            <label className="text-sm font-medium mb-2 block">Tax Rate (%)</label>
            <Input
              type="number"
              step="0.01"
              min="0"
              max="100"
              value={formData.taxRate}
              onChange={e =>
                setFormData(prev => ({ ...prev, taxRate: Number(e.target.value) || 0 }))
              }
              autoComplete="off"
            />
          </div>
          <div className="pt-2">
            <div className="text-sm font-medium mb-2">Total (excl. tax)</div>
            <div className="text-right font-medium">
              {formatCurrency(Number(formData.quantity) * Number(formData.unitPrice), currency)}
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={!formData.product.trim()}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

const InvoiceLineItemDisplay = ({
  item,
  index,
  currency,
  onRemove,
  onEdit,
}: {
  item: InvoiceLineSchema
  index: number
  currency: string
  onRemove: (index: number) => void
  onEdit: (index: number) => void
}) => {
  return (
    <div className="py-2">
      <div className="flex justify-between items-start">
        <div className="flex-1">
          <div className="flex items-center gap-2">
            <div className="text-[13px] font-medium">{item.product}</div>
          </div>
          {item.startDate && item.endDate && (
            <div className="text-[11px] text-muted-foreground mt-1">
              {item.startDate.toLocaleDateString()} → {item.endDate.toLocaleDateString()}
            </div>
          )}
        </div>
        <div className="text-right flex items-center gap-2">
          <div>
            <div className="text-[11px] text-muted-foreground">
              {item.quantity} × {formatCurrency(item.unitPrice, currency)}
            </div>
            <div className="text-[13px] font-medium">
              {formatCurrency(Number(item.quantity) * Number(item.unitPrice), currency)}
            </div>
          </div>
          <div className="flex items-center gap-1">
            <Button
              type="button"
              variant="link"
              size="icon"
              onClick={() => onEdit(index)}
              className="h-6 w-6 p-0"
            >
              <Edit size={14} />
            </Button>
            <Button
              type="button"
              variant="link"
              size="icon"
              onClick={() => onRemove(index)}
              className="h-6 w-6 p-0"
            >
              <XIcon size={14} />
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}

const InvoiceLineTable = ({
  methods,
  currency,
}: {
  methods: ReturnType<typeof useZodForm<typeof schemas.invoices.createInvoiceSchema>>
  currency: string
}) => {
  const { fields, append, remove, update } = useFieldArray({
    control: methods.control,
    name: 'lines',
  })
  const [isModalOpen, setIsModalOpen] = useState(false)
  const [isEditModalOpen, setIsEditModalOpen] = useState(false)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)

  const addInvoiceLine = useCallback(
    (item: InvoiceLineSchema) => {
      append(item)
    },
    [append]
  )

  const editInvoiceLine = useCallback((index: number) => {
    setEditingIndex(index)
    setIsEditModalOpen(true)
  }, [])

  const saveEditedLine = useCallback(
    (item: InvoiceLineSchema) => {
      if (editingIndex !== null) {
        update(editingIndex, item)
        setEditingIndex(null)
      }
    },
    [editingIndex, update]
  )

  return (
    <>
      <div className="space-y-2">
        {fields.map((field, index) => (
          <InvoiceLineItemDisplay
            key={field.id}
            item={field}
            index={index}
            currency={currency}
            onRemove={remove}
            onEdit={editInvoiceLine}
          />
        ))}
      </div>

      <Button type="button" variant="link" onClick={() => setIsModalOpen(true)} className="mt-2">
        + Add Line
      </Button>

      <AddLineItemModal
        isOpen={isModalOpen}
        onClose={() => setIsModalOpen(false)}
        onAdd={addInvoiceLine}
        currency={currency}
      />

      {editingIndex !== null && (
        <EditLineItemModal
          isOpen={isEditModalOpen}
          onClose={() => {
            setIsEditModalOpen(false)
            setEditingIndex(null)
          }}
          onSave={saveEditedLine}
          currency={currency}
          initialData={fields[editingIndex]}
        />
      )}
    </>
  )
}

const CreateInvoicePreview = ({
  methods,
}: {
  methods: ReturnType<typeof useZodForm<typeof schemas.invoices.createInvoiceSchema>>
}) => {
  const watchedCustomerId = methods.watch('customerId')
  const watchedLines = methods.watch('lines')
  const watchedCurrency = methods.watch('currency')
  const watchedInvoiceDate = methods.watch('invoiceDate')
  const watchedDueDate = methods.watch('dueDate')
  const watchedDiscount = methods.watch('discount')
  const watchedPurchaseOrder = methods.watch('purchaseOrder')

  const [previewSvgs, setPreviewSvgs] = useState<string[]>([])
  const previewInvoiceMutation = useMutation(previewNewInvoiceSvg)

  const generatePreview = useCallback(async () => {
    if (!watchedCustomerId || !watchedCurrency || !watchedInvoiceDate || !watchedLines?.length) {
      setPreviewSvgs([])
      return
    }

    try {
      const response = await previewInvoiceMutation.mutateAsync({
        invoice: {
          customerId: watchedCustomerId,
          invoiceDate: mapDatev2(watchedInvoiceDate),
          dueDate: watchedDueDate ? mapDatev2(watchedDueDate) : undefined,
          currency: watchedCurrency,
          purchaseOrder: methods.getValues('purchaseOrder') || undefined,
          discount: watchedDiscount ? watchedDiscount.toString() : undefined,
          lineItems: watchedLines?.map(line => ({
            product: line.product,
            startDate: mapDatev2(line.startDate),
            endDate: mapDatev2(line.endDate),
            quantity: line.quantity.toString(),
            unitPrice: line.unitPrice.toString(),
            taxRate: ((line.taxRate || 0) / 100).toString(),
          })),
        },
      })

      const svgContents =
        response.svgs.map(svg => {
          const scaledHtml = svg ? resizeSvgContent(svg, 1) : ''

          // Extract just the SVG from the HTML
          const parser = new DOMParser()
          const doc = parser.parseFromString(scaledHtml, 'text/html')
          const svgElement = doc.querySelector('svg')
          return svgElement?.outerHTML || ''
        }) ?? []

      setPreviewSvgs(svgContents || [])
    } catch (error) {
      setPreviewSvgs([])
    }
  }, [
    watchedCustomerId,
    watchedLines,
    watchedDiscount,
    watchedCurrency,
    watchedInvoiceDate,
    watchedDueDate,
    watchedPurchaseOrder,
  ])

  useEffect(() => {
    const timeoutId = setTimeout(generatePreview, 500)
    return () => clearTimeout(timeoutId)
  }, [generatePreview])

  if (!watchedCustomerId || !watchedCurrency || !watchedInvoiceDate || !watchedLines?.length) {
    return (
      <div className="h-full flex items-center justify-center bg-gray-50 rounded-lg border-2 border-dashed border-gray-300">
        <div className="text-center">
          <div className="text-lg font-medium text-gray-500 mb-2">Invoice Preview</div>
          <div className="text-sm text-gray-400">
            Fill in the form on the left to preview the invoice
          </div>
        </div>
      </div>
    )
  }

  if (previewInvoiceMutation.isPending) {
    return (
      <div className="h-full flex items-center justify-center bg-white rounded-lg border">
        <div className="text-sm text-muted-foreground">Generating preview...</div>
      </div>
    )
  }

  return (
    <div className="w-full h-full flex flex-col">
      <div
        className="flex flex-col items-center justify-center gap-5 bg-gray-100 py-10 relative"
        style={{ minHeight: 'fit-content' }}
      >
        {previewSvgs.map((svgContent, i) => (
          <div
            className="  bg-white"
            key={`svg-${i}`}
            style={{
              boxShadow: '0px 4px 12px rgba(89, 85, 101, .2)',
            }}
            dangerouslySetInnerHTML={{ __html: svgContent }}
          />
        ))}
      </div>
    </div>
  )
}

export const InvoiceCreate = () => {
  // make taxRate a number instead of text and prepopulate it with customer tax (see TaxEngine and expose it in grpc api)
  const navigate = useNavigate()
  const basePath = useBasePath()
  const queryClient = useQueryClient()
  const [searchParams] = useSearchParams()

  const activeCurrenciesQuery = useQuery(listTenantCurrencies)
  const activeCurrencies = activeCurrenciesQuery.data?.currencies ?? []

  const customerIdFromQuery = searchParams.get('customerId')

  const defaultValues = {
    invoiceDate: new Date(),
    lines: [],
    discount: 0,
    ...(customerIdFromQuery && customerIdFromQuery.trim() !== ''
      ? { customerId: customerIdFromQuery }
      : {}),
  }

  const methods = useZodForm({
    schema: schemas.invoices.createInvoiceSchema,
    defaultValues: defaultValues,
  })

  const watchedCustomerId = methods.watch('customerId')
  const watchedInvoiceDate = methods.watch('invoiceDate')

  const prevCustomerIdRef = useRef(watchedCustomerId)
  const [showConfirmDialog, setShowConfirmDialog] = useState(false)
  const [pendingCustomerId, setPendingCustomerId] = useState<string | null>(null)

  const handleCustomerChange = (newCustomerId: string) => {
    const currentCustomerId = methods.getValues('customerId')
    const currentCurrency = methods.getValues('currency')
    const currentDueDate = methods.getValues('dueDate')

    // If no customer is selected yet, or if no dependent fields are filled, just change directly
    if (!currentCustomerId || (!currentCurrency && !currentDueDate)) {
      methods.setValue('customerId', newCustomerId)
      return
    }

    // Show confirmation dialog
    setPendingCustomerId(newCustomerId)
    setShowConfirmDialog(true)
  }

  const confirmCustomerChange = () => {
    if (pendingCustomerId) {
      methods.setValue('customerId', pendingCustomerId)
      setPendingCustomerId(null)
    }
    setShowConfirmDialog(false)
  }

  const cancelCustomerChange = () => {
    setPendingCustomerId(null)
    setShowConfirmDialog(false)
  }

  const onSubmit = async (data: z.infer<typeof schemas.invoices.createInvoiceSchema>) => {
    try {
      const res = await createInvoiceMutation.mutateAsync({
        invoice: {
          customerId: data.customerId,
          invoiceDate: mapDatev2(data.invoiceDate),
          dueDate: data.dueDate ? mapDatev2(data.dueDate) : undefined,
          currency: data.currency,
          purchaseOrder: data.purchaseOrder,
          discount: data.discount ? data.discount.toString() : undefined,
          lineItems: data.lines?.map(line => ({
            product: line.product,
            startDate: mapDatev2(line.startDate),
            endDate: mapDatev2(line.endDate),
            quantity: line.quantity.toString(),
            unitPrice: line.unitPrice.toString(),
            taxRate: ((line.taxRate || 0) / 100).toString(),
          })),
        },
      })
      toast.success('Invoice created')
      navigate(`${basePath}/invoices/${res.invoice?.id}`)
    } catch (error) {
      toast.error('Failed to create invoice')
      console.error(error)
    }
  }

  const customerQuery = useQuery(
    getCustomerById,
    {
      id: watchedCustomerId ?? '',
    },
    { enabled: Boolean(watchedCustomerId) }
  )

  const invoicingEntityQuery = useQuery(
    getInvoicingEntity,
    {
      id: customerQuery.data?.customer?.invoicingEntityId ?? '',
    },
    { enabled: Boolean(customerQuery.data?.customer?.invoicingEntityId) }
  )

  useEffect(() => {
    if (watchedInvoiceDate && invoicingEntityQuery.data?.entity?.netTerms) {
      const netTerms = invoicingEntityQuery.data.entity.netTerms
      const dueDate = new Date(watchedInvoiceDate)
      dueDate.setDate(dueDate.getDate() + netTerms)
      methods.setValue('dueDate', dueDate)
    }

    // Auto-set currency when customer changes or when customer is initially loaded from query param
    if (
      customerQuery.data?.customer?.currency &&
      (watchedCustomerId !== prevCustomerIdRef.current || !methods.getValues('currency'))
    ) {
      methods.setValue('currency', customerQuery.data.customer.currency)
      prevCustomerIdRef.current = watchedCustomerId
    }
  }, [
    watchedInvoiceDate,
    watchedCustomerId,
    customerQuery.data?.customer?.invoicingEntityId,
    customerQuery.data?.customer?.currency,
    invoicingEntityQuery.data?.entity?.netTerms,
  ])

  const createInvoiceMutation = useMutation(createInvoice, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: [listInvoices.service.typeName] })
    },
  })

  return (
    <>
      <PageHeading>Create invoice</PageHeading>
      <Flex className="h-full">
        {/* Left Panel - Invoice Form */}
        <Flex direction="column" className="w-1/3 border-r border-border">
          <div className="flex-1 overflow-auto p-6">
            <Form {...methods}>
              <form onSubmit={methods.handleSubmit(onSubmit)}>
                <div className="flex flex-col gap-4">
                  <GenericFormField
                    control={methods.control}
                    layout="horizontal"
                    label="Customer"
                    name="customerId"
                    render={({ field }) => (
                      <CustomerSelect value={field.value} onChange={handleCustomerChange} />
                    )}
                  />
                  <GenericFormField
                    control={methods.control}
                    layout="horizontal"
                    label="Currency"
                    name="currency"
                    render={({ field }) => (
                      <Select onValueChange={field.onChange} value={field.value}>
                        <SelectTrigger className="min-w-[13em]">
                          <SelectValue placeholder="Select a currency" />
                        </SelectTrigger>
                        <SelectContent>
                          {activeCurrencies.map((a, i) => (
                            <SelectItem value={a} key={`item` + i}>
                              {a}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    )}
                  />
                  <GenericFormField
                    control={methods.control}
                    layout="horizontal"
                    label="Invoice date"
                    name="invoiceDate"
                    render={({ field }) => (
                      <DatePicker
                        mode="single"
                        captionLayout="dropdown"
                        className="min-w-[13em]"
                        date={field.value}
                        onSelect={field.onChange}
                      />
                    )}
                  />
                  <GenericFormField
                    control={methods.control}
                    layout="horizontal"
                    label="Due date"
                    name="dueDate"
                    render={({ field }) => (
                      <DatePicker
                        mode="single"
                        captionLayout="dropdown"
                        className="min-w-[13em]"
                        date={field.value}
                        onSelect={field.onChange}
                      />
                    )}
                  />
                  <GenericFormField
                    control={methods.control}
                    layout="horizontal"
                    label="Purchase Order"
                    name="purchaseOrder"
                    render={({ field }) => (
                      <Input
                        {...field}
                        placeholder="Order № (optional)"
                        className="min-w-[13em]"
                        autoComplete="off"
                      />
                    )}
                  />

                  <div className="space-y-2 border-t pt-4">
                    <div>
                      <h3 className="text-lg font-medium">Line Items</h3>
                      {methods.formState.errors.lines && (
                        <div className="text-[0.8rem] font-medium text-destructive mt-1">
                          {methods.formState.errors.lines.message}
                        </div>
                      )}
                    </div>

                    <InvoiceLineTable
                      methods={methods}
                      currency={methods.watch('currency') || 'USD'}
                    />

                    <div className="border-t pt-4">
                      <GenericFormField
                        control={methods.control}
                        layout="horizontal"
                        label="Discount"
                        name="discount"
                        render={({ field }) => (
                          <UncontrolledPriceInput
                            {...field}
                            currency={methods.watch('currency') || 'USD'}
                            showCurrency={false}
                            className="min-w-[13em]"
                            step="0.1"
                            precision={2}
                            onChange={e => field.onChange(Number(e.target.value) || 0)}
                            autoComplete="off"
                          />
                        )}
                      />
                    </div>
                  </div>

                  <div className="flex gap-2 pt-4">
                    <Link to={`${basePath}/invoices`}>
                      <Button type="button" variant="secondary" title="Cancel">
                        Cancel
                      </Button>
                    </Link>
                    <Button
                      type="submit"
                      variant="primary"
                      disabled={createInvoiceMutation.isPending}
                    >
                      {createInvoiceMutation.isPending ? 'Creating...' : 'Create'}
                    </Button>
                  </div>
                </div>
              </form>
            </Form>
          </div>
        </Flex>

        {/* Right Panel - Invoice Preview */}
        <div className="w-2/3 flex flex-col">
          <div className="flex-1 overflow-auto p-6">
            <CreateInvoicePreview methods={methods} />
          </div>
        </div>
      </Flex>

      <AlertDialog open={showConfirmDialog} onOpenChange={setShowConfirmDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Confirm Customer Change</AlertDialogTitle>
            <AlertDialogDescription>
              Changing the customer will reset the currency and due date to match the new
              customer&apos;s settings. Are you sure you want to continue?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={cancelCustomerChange}>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={confirmCustomerChange}>Continue</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
