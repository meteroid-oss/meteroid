import { useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef } from '@tanstack/react-table'
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
  Form,
  GenericFormField,
  Input,
  Select, SelectContent, SelectItem,
  SelectTrigger, SelectValue
} from "@ui/components";
import { XIcon } from "lucide-react";
import { useEffect, useRef, useState, useCallback, useMemo, memo } from "react";
import { useFieldArray, useWatch, Control } from 'react-hook-form'
import { Link, useNavigate } from "react-router-dom";
import { toast } from 'sonner'
import { z } from 'zod'

import PageHeading from "@/components/PageHeading/PageHeading";
import { UncontrolledPriceInput } from "@/components/form/PriceInput";
import { SimpleTable } from "@/components/table/SimpleTable";
import { CustomerSelect } from "@/features/customers/CustomerSelect";
import { DatePickerWithRange } from "@/features/dashboard/DateRangePicker";
import { useBasePath } from "@/hooks/useBasePath";
import { useZodForm } from "@/hooks/useZodForm";
import { useQuery } from "@/lib/connectrpc";
import { mapDatev2 } from "@/lib/mapping";
import { schemas } from "@/lib/schemas";
import { InvoiceLineSchema } from "@/lib/schemas/invoices";
import { getCustomerById } from "@/rpc/api/customers/v1/customers-CustomersService_connectquery";
import {
  createInvoice,
  listInvoices,
  previewNewInvoiceHtml
} from "@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery";
import {
  getInvoicingEntity
} from "@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery";
import { listTenantCurrencies } from "@/rpc/api/tenants/v1/tenants-TenantsService_connectquery";


const formatCurrency = (amount: number, currency: string) => {
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: currency || 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(amount)
}

const TotalCell = memo(({ control, rowIndex, currency }: {
  control: Control<z.infer<typeof schemas.invoices.createInvoiceSchema>>,
  rowIndex: number,
  currency: string
}) => {
  const quantity = useWatch({ control, name: `lines.${rowIndex}.quantity` }) || 0
  const unitPrice = useWatch({ control, name: `lines.${rowIndex}.unitPrice` }) || 0

  return (
    <div className="text-right font-medium">
      {formatCurrency(Number(quantity) * Number(unitPrice), currency)}
    </div>
  )
})

const DateRangeCell = memo(({ control, rowIndex, methods }: {
  control: Control<z.infer<typeof schemas.invoices.createInvoiceSchema>>,
  rowIndex: number,
  methods: ReturnType<typeof useZodForm<typeof schemas.invoices.createInvoiceSchema>>
}) => {
  const startDate = useWatch({ control, name: `lines.${rowIndex}.startDate` })
  const endDate = useWatch({ control, name: `lines.${rowIndex}.endDate` })

  return (
    <DatePickerWithRange
      range={{ from: startDate, to: endDate }}
      setRange={(dateRange) => {
        const newStartDate = dateRange?.from || new Date()
        let newEndDate = dateRange?.to || new Date()

        // If endDate is not after startDate, set it to startDate + 1 day
        if (newEndDate <= newStartDate) {
          newEndDate = new Date(newStartDate)
          newEndDate.setDate(newEndDate.getDate() + 1)
        }

        methods.setValue(`lines.${rowIndex}.startDate`, newStartDate)
        methods.setValue(`lines.${rowIndex}.endDate`, newEndDate)
      }}
    />
  )
})

const InvoiceLineTable = ({
  methods,
  currency,
}: {
  methods: ReturnType<typeof useZodForm<typeof schemas.invoices.createInvoiceSchema>>
  currency: string
}) => {
  const { fields, append, remove } = useFieldArray({
    control: methods.control,
    name: 'lines',
  })

  const addInvoiceLine = useCallback(() => {
    const startDate = new Date()
    const endDate = new Date(startDate)
    endDate.setDate(endDate.getDate() + 1)

    append({
      product: '',
      startDate,
      endDate,
      quantity: 1.00,
      unitPrice: 1.00,
      taxRate: 20.00,
    })
  }, [append])

  const columns = useMemo<ColumnDef<InvoiceLineSchema>[]>(
    () => [
      {
        header: 'Product',
        cell: ({ row }) => (
          <GenericFormField
            control={methods.control}
            name={`lines.${row.index}.product`}
            render={({ field }) => (
              <Input
                {...field}
                placeholder="Product name"
                autoComplete="off"
              />
            )}
          />
        ),
      },
      {
        header: 'Date Range',
        cell: ({ row }) => (
          <DateRangeCell
            control={methods.control}
            rowIndex={row.index}
            methods={methods}
          />
        ),
      },
      {
        header: 'Quantity',
        cell: ({ row }) => (
          <GenericFormField
            control={methods.control}
            name={`lines.${row.index}.quantity`}
            render={({ field }) => (
              <Input
                {...field}
                type="number"
                step="0.01"
                min="0.01"
                value={field.value || 0}
                onChange={(e) => field.onChange(Number(e.target.value) || 0.0)}
                autoComplete="off"
              />
            )}
          />
        ),
      },
      {
        header: 'Unit Price',
        cell: ({ row }) => (
          <GenericFormField
            control={methods.control}
            name={`lines.${row.index}.unitPrice`}
            render={({ field }) => (
              <UncontrolledPriceInput
                {...field}
                currency={currency}
                showCurrency={false}
                className="max-w-xs"
                precision={2}
                onChange={(e) => field.onChange(Number(e.target.value) || 0.0)}
                autoComplete="off"
              />
            )}
          />
        ),
      },
      {
        header: 'Tax Rate (%)',
        cell: ({ row }) => (
          <GenericFormField
            control={methods.control}
            name={`lines.${row.index}.taxRate`}
            render={({ field }) => (
              <Input
                {...field}
                type="number"
                step="0.01"
                min="0"
                max="100"
                value={field.value || 0.00}
                onChange={(e) => field.onChange(Number(e.target.value) || 0)}
                autoComplete="off"
              />
            )}
          />
        ),
      },
      {
        header: 'Total (excl. tax)',
        cell: ({ row }) => <TotalCell control={methods.control} rowIndex={row.index} currency={currency}/>,
      },
      {
        header: '',
        id: 'remove',
        cell: ({ row }) => (
          <Button
            type="button"
            variant="link"
            size="icon"
            onClick={() => remove(row.index)}
          >
            <XIcon size={16}/>
          </Button>
        ),
      },
    ],
    [methods.control, remove, currency]
  )

  return (
    <>
      {fields.length > 0 ? (
        <>
          <SimpleTable columns={columns} data={fields}/>
          <Button variant="link" onClick={addInvoiceLine}>
            + Add Line
          </Button>
        </>
      ) : (
        <Button variant="link" onClick={addInvoiceLine}>
          + Add Line
        </Button>
      )}
    </>
  )
}

export const InvoiceCreate = () => {
  // add discount
  // make taxRate a number instead of text and prepopulate it with customer tax (see TaxEngine and expose it in grpc api)
  const navigate = useNavigate()
  const basePath = useBasePath()
  const queryClient = useQueryClient()

  const activeCurrenciesQuery = useQuery(listTenantCurrencies)
  const activeCurrencies = activeCurrenciesQuery.data?.currencies ?? []

  const defaultValues = {
    invoiceDate: new Date(),
    lines: [],
    discount: 0,
  }

  const methods = useZodForm({
    schema: schemas.invoices.createInvoiceSchema,
    defaultValues: defaultValues,
  })

  const watchedCustomerId = methods.watch('customerId')
  const watchedInvoiceDate = methods.watch('invoiceDate')
  const watchedLines = methods.watch('lines')

  const prevCustomerIdRef = useRef(watchedCustomerId)
  const [showConfirmDialog, setShowConfirmDialog] = useState(false)
  const [pendingCustomerId, setPendingCustomerId] = useState<string | null>(null)
  const [showPreviewModal, setShowPreviewModal] = useState(false)
  const [previewHtml, setPreviewHtml] = useState<string>('')

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
          }))
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

    // Only auto-set currency when customer changes
    if (customerQuery.data?.customer?.currency &&
      watchedCustomerId !== prevCustomerIdRef.current) {
      methods.setValue('currency', customerQuery.data.customer.currency)
      prevCustomerIdRef.current = watchedCustomerId
    }
  }, [
    watchedInvoiceDate,
    watchedCustomerId,
    customerQuery.data?.customer?.invoicingEntityId,
    customerQuery.data?.customer?.currency,
    invoicingEntityQuery.data?.entity?.netTerms,
    methods
  ])

  const createInvoiceMutation = useMutation(createInvoice, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: [listInvoices.service.typeName] })
    },
  })

  const previewInvoiceMutation = useMutation(previewNewInvoiceHtml)

  const handlePreview = async () => {
    const formData = methods.getValues()
    const isValid = await methods.trigger() // Validate the form

    if (!isValid) {
      toast.error('Please fix form errors before previewing')
      return
    }

    try {
      const response = await previewInvoiceMutation.mutateAsync({
        invoice: {
          customerId: formData.customerId,
          invoiceDate: mapDatev2(formData.invoiceDate),
          dueDate: formData.dueDate ? mapDatev2(formData.dueDate) : undefined,
          currency: formData.currency,
          purchaseOrder: formData.purchaseOrder || undefined,
          discount: formData.discount ? formData.discount.toString() : undefined,
          lineItems: formData.lines?.map(line => ({
            product: line.product,
            startDate: mapDatev2(line.startDate),
            endDate: mapDatev2(line.endDate),
            quantity: line.quantity.toString(),
            unitPrice: line.unitPrice.toString(),
            taxRate: ((line.taxRate || 0) / 100).toString(),
          }))
        },
      })

      setPreviewHtml(response.html || '')
      setShowPreviewModal(true)
    } catch (error) {
      toast.error('Failed to generate preview')
      console.error(error)
    }
  }

  return (
    <>
      <PageHeading>Create invoice</PageHeading>
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)}>
          <div className="flex flex-col gap-4 max-w-6xl">
            <GenericFormField
              control={methods.control}
              layout="horizontal"
              label="Customer"
              name="customerId"
              render={({ field }) => (
                <CustomerSelect value={field.value} onChange={handleCustomerChange}/>
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
                    <SelectValue placeholder="Select a currency"/>
                  </SelectTrigger>
                  <SelectContent>
                    {
                      activeCurrencies.map((a, i) => <SelectItem value={a} key={`item` + i}>{a}</SelectItem>)
                    }
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
                />
              )}
            />

            <div className="space-y-4">
              <div>
                <h3 className="text-lg font-medium">Invoice Lines</h3>
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

              {watchedLines?.length > 0 && (() => {
                const currency = methods.watch('currency') || 'USD'
                const discount = Number(methods.watch('discount')) || 0
                const subtotal = watchedLines.reduce((sum, line) => {
                  const quantity = Number(line?.quantity) || 0
                  const unitPrice = Number(line?.unitPrice) || 0
                  return sum + (quantity * unitPrice)
                }, 0)
                const discountedSubtotal = Math.max(0, subtotal - discount)
                const totalTax = watchedLines.reduce((sum, line) => {
                  const quantity = Number(line?.quantity) || 0
                  const unitPrice = Number(line?.unitPrice) || 0
                  const taxRate = Number(line?.taxRate) || 0
                  const lineSubtotal = quantity * unitPrice
                  const lineDiscountProportion = subtotal > 0 ? lineSubtotal / subtotal : 0
                  const lineDiscountAmount = discount * lineDiscountProportion
                  const discountedLineAmount = Math.max(0, lineSubtotal - lineDiscountAmount)
                  return sum + (discountedLineAmount * (taxRate / 100))
                }, 0)
                const total = discountedSubtotal + totalTax

                return (
                  <div className="flex justify-end">
                    <div className="w-80 space-y-2 border-t pt-4">
                      <div className="flex justify-between">
                        <span>Subtotal:</span>
                        <span>{formatCurrency(subtotal, currency)}</span>
                      </div>
                      <div className="flex justify-between">
                        <span>Discount:</span>
                        <div className="flex items-center gap-2">
                          <GenericFormField
                            control={methods.control}
                            name="discount"
                            render={({ field }) => (
                              <UncontrolledPriceInput
                                {...field}
                                currency={currency}
                                showCurrency={false}
                                className="w-26 text-right"
                                precision={2}
                                onChange={(e) => field.onChange(Number(e.target.value) || 0)}
                                autoComplete="off"
                              />
                            )}
                          />
                        </div>
                      </div>
                      <div className="flex justify-between">
                        <span>Tax:</span>
                        <span>{formatCurrency(totalTax, currency)}</span>
                      </div>
                      <div className="flex justify-between font-semibold border-t pt-2">
                        <span>Total:</span>
                        <span>{formatCurrency(total, currency)}</span>
                      </div>
                    </div>
                  </div>
                )
              })()}
            </div>

            <div className="flex gap-2">
              <Link to={`${basePath}/invoices`}>
                <Button
                  type="button"
                  variant="secondary"
                  title="Cancel"
                >
                  Cancel
                </Button>
              </Link>
              <Button
                type="button"
                variant="outline"
                onClick={handlePreview}
                disabled={previewInvoiceMutation.isPending}
              >
                {previewInvoiceMutation.isPending ? 'Loading...' : 'Preview'}
              </Button>
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

      <AlertDialog open={showConfirmDialog} onOpenChange={setShowConfirmDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Confirm Customer Change</AlertDialogTitle>
            <AlertDialogDescription>
              Changing the customer will reset the currency and due date to match the new customer&apos;s settings.
              Are you sure you want to continue?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={cancelCustomerChange}>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={confirmCustomerChange}>Continue</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <Dialog open={showPreviewModal} onOpenChange={setShowPreviewModal}>
        <DialogContent className="w-full min-h-[870px] max-w-[824px] p-2 bg-muted">
          {previewInvoiceMutation.isPending ? (
            <div className="flex items-center justify-center h-full">
              <span>Loading preview...</span>
            </div>
          ) : (
            <iframe
              srcDoc={previewHtml}
              className="w-full h-full border border-border rounded-sm bg-white mt-12"
              title="Invoice Preview"
            />
          )}
        </DialogContent>
      </Dialog>
    </>
  )
}

