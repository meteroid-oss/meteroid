import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Modal,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { Combobox } from '@/components/Combobox'
import ConfirmationModal from '@/components/ConfirmationModal'
import { Loading } from '@/components/Loading'
import { CustomerFormFields } from '@/features/customers/form/CustomerFormFields'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import { CreateCustomerSchema } from '@/lib/schemas/customers'
import {
  createCustomer,
  listCustomers,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { listInvoicingEntities } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

interface CustomersCreatePanelProps {
  visible: boolean
  closePanel: () => void
}

export const CustomersCreatePanel = ({ visible, closePanel }: CustomersCreatePanelProps) => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [isClosingPanel, setIsClosingPanel] = useState(false)

  const createCustomerMut = useMutation(createCustomer, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listCustomers.service.typeName] })
    },
  })

  const activeCurrenciesQuery = useQuery(listTenantCurrencies)
  const activeCurrencies = activeCurrenciesQuery.data?.currencies ?? []

  const listInvoicingEntitiesQuery = useQuery(listInvoicingEntities)

  const methods = useZodForm({
    schema: schemas.customers.createCustomerSchema,
    defaultValues: {
      customTaxes: [],
      isTaxExempt: false,
    },
  })

  // Set default invoicing entity when loaded
  useEffect(() => {
    if (listInvoicingEntitiesQuery.data?.entities) {
      const defaultEntity = listInvoicingEntitiesQuery.data.entities.find(
        entity => entity.isDefault
      )
      if (defaultEntity) {
        methods.setValue('invoicingEntity', defaultEntity.id)
      }
    }
  }, [listInvoicingEntitiesQuery.data?.entities, methods])

  const safeClosePanel = () => {
    const isDirty = methods.formState.isDirty
    if (isDirty) {
      setIsClosingPanel(true)
    } else {
      methods.reset()
      closePanel()
    }
  }

  const handleConfirmClose = () => {
    setIsClosingPanel(false)
    methods.reset()
    closePanel()
  }

  const onSubmit = async (values: CreateCustomerSchema) => {
    const res = await createCustomerMut.mutateAsync({
      data: {
        name: values.name,
        alias: values.alias || undefined,
        billingEmail: values.email || undefined,
        invoicingEmails: values.invoicingEmail ? [values.invoicingEmail] : [],
        phone: values.phone || undefined,
        currency: values.currency,
        invoicingEntityId: values.invoicingEntity,
        vatNumber: values.vatNumber || undefined,
        customTaxes: (values.customTaxes || []).map(tax => ({
          taxCode: tax.taxCode,
          name: tax.name,
          rate: (tax.rate / 100).toString(),
        })),
        isTaxExempt: values.isTaxExempt,
        billingAddress: values.billingAddress,
        shippingAddress: values.shippingAddress,
      },
    })
    if (res.customer?.id) {
      navigate(`./${res.customer.id}`)
    }
  }

  const isLoading = activeCurrenciesQuery.isLoading || listInvoicingEntitiesQuery.isLoading

  return (
    <>
      <Sheet open={visible} onOpenChange={safeClosePanel}>
        <SheetContent size="medium" side="right" className="p-0">
          <Form {...methods}>
            <form onSubmit={methods.handleSubmit(onSubmit)} className="flex h-full w-full flex-col">
              <SheetHeader className="ml-6 mt-4">
                <SheetTitle>Create customer</SheetTitle>
              </SheetHeader>

              <div className="flex-1 overflow-y-auto">
                <div className="space-y-6 p-6">
                  {isLoading ? (
                    <Loading />
                  ) : (
                    <>
                      {/* Invoicing Entity & Currency - only shown on create */}
                      <div className="space-y-4">
                        <h3 className="font-semibold">Account settings</h3>
                        <FormField
                          control={methods.control}
                          name="invoicingEntity"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>Invoicing entity</FormLabel>
                              <Combobox
                                placeholder="Select invoicing entity..."
                                value={field.value}
                                onChange={field.onChange}
                                options={
                                  listInvoicingEntitiesQuery.data?.entities?.map(entity => ({
                                    label: (
                                      <div className="flex flex-row w-full">
                                        <div className="pr-2">
                                          {getCountryFlagEmoji(entity.country)}
                                        </div>
                                        <div>{entity.legalName}</div>
                                        <div className="flex-grow" />
                                        {entity.isDefault && (
                                          <Badge variant="primary" size="sm">
                                            Default
                                          </Badge>
                                        )}
                                      </div>
                                    ),
                                    value: entity.id,
                                  })) ?? []
                                }
                              />
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                        <FormField
                          control={methods.control}
                          name="currency"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>
                                Currency <span className="text-destructive">*</span>
                              </FormLabel>
                              <Select onValueChange={field.onChange} defaultValue={field.value}>
                                <FormControl>
                                  <SelectTrigger>
                                    <SelectValue placeholder="Select a currency" />
                                  </SelectTrigger>
                                </FormControl>
                                <SelectContent>
                                  {activeCurrencies.map((a, i) => (
                                    <SelectItem value={a} key={`item` + i}>
                                      {a}
                                    </SelectItem>
                                  ))}
                                </SelectContent>
                              </Select>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                      </div>

                      {/* Shared customer form fields */}
                      <CustomerFormFields control={methods.control} />
                    </>
                  )}
                </div>
              </div>

              <SheetFooter className="border-t border-border p-3">
                <Button variant="outline" type="reset" onClick={safeClosePanel}>
                  Cancel
                </Button>
                <Button type="submit">Create customer</Button>
              </SheetFooter>
            </form>
          </Form>
        </SheetContent>
      </Sheet>

      <ConfirmationModal
        visible={isClosingPanel}
        header="Confirm to close"
        buttonLabel="Confirm"
        onSelectCancel={() => setIsClosingPanel(false)}
        onSelectConfirm={handleConfirmClose}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-muted-foreground">
            There are unsaved changes. Are you sure you want to close the panel? Your changes will
            be lost.
          </p>
        </Modal.Content>
      </ConfirmationModal>
    </>
  )
}
