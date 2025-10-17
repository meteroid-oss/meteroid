import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  CheckboxFormField,
  Flex,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  InputFormField,
  Modal,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Minus, Plus, X } from 'lucide-react'
import { ComponentProps, useEffect, useState } from 'react'
import { useFieldArray } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import { customerSchema } from '@/features/customers/cards/customer/schema'
import { useZodForm } from '@/hooks/useZodForm'
import {
  getCustomerById,
  updateCustomer,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof Modal>, 'visible' | 'onCancel'> & {
  customer: Customer
}

export const EditCustomerModal = ({ customer, visible, onCancel }: Props) => {
  const queryClient = useQueryClient()
  const [showShippingAddress, setShowShippingAddress] = useState(
    Boolean(customer.shippingAddress?.address)
  )

  const updateCustomerMutation = useMutation(updateCustomer, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getCustomerById, { id: customer.id }),
      })
    },
  })

  const getDefaultValues = () => ({
    name: customer.name,
    alias: customer.alias,
    email: customer.billingEmail,
    invoicingEmail: customer.invoicingEmails[0],
    phone: customer.phone,
    vatNumber: customer.vatNumber,
    customTaxes:
      customer.customTaxes?.map(tax => ({
        taxCode: tax.taxCode,
        name: tax.name,
        rate: Number(tax.rate) * 100,
      })) ?? [],
    isTaxExempt: customer.isTaxExempt,
    billingAddress: customer.billingAddress,
    shippingAddress: customer.shippingAddress,
  })

  const methods = useZodForm({
    mode: 'onChange',
    schema: customerSchema,
    defaultValues: getDefaultValues(),
  })

  useEffect(() => {
    if (visible) {
      methods.reset(getDefaultValues())
      setShowShippingAddress(Boolean(customer.shippingAddress?.address))
    }
  }, [visible, customer])

  const { fields, append, remove } = useFieldArray({
    control: methods.control,
    name: 'customTaxes',
  })

  const customTaxes = methods.watch('customTaxes')
  const hasCustomTaxes = customTaxes && customTaxes.length > 0

  // Reset isTaxExempt to false when custom taxes are present
  useEffect(() => {
    if (hasCustomTaxes) {
      methods.setValue('isTaxExempt', false)
    }
  }, [hasCustomTaxes, methods])

  const onSubmit = async (data: z.infer<typeof customerSchema>) => {
    await updateCustomerMutation.mutateAsync({
      customer: {
        id: customer.id,
        name: data.name,
        alias: data.alias,
        billingEmail: data.email,
        // TODO allow multiple
        invoicingEmails: data.invoicingEmail ? { emails: [data.invoicingEmail] } : undefined,
        phone: data.phone,
        vatNumber: data.vatNumber,
        customTaxes: {
          taxes: (data.customTaxes || []).map(tax => ({
            taxCode: tax.taxCode,
            name: tax.name,
            rate: (tax.rate / 100).toString(),
          })),
        },
        isTaxExempt: data.isTaxExempt,
        billingAddress: data.billingAddress,
        shippingAddress: data.shippingAddress,
      },
    })
    toast.success('Customer updated')
    onCancel?.()
  }

  return (
    <Modal
      header={<>Edit customer</>}
      visible={visible}
      onCancel={onCancel}
      onConfirm={() => methods.handleSubmit(onSubmit)()}
      size="large"
    >
      <Modal.Content>
        <Form {...methods}>
          <form className="max-h-[70vh] overflow-y-auto pr-2">
            <div className="py-4 w-full space-y-6">
              {/* Customer Details Section */}
              <div className="space-y-4">
                <h3 className="font-semibold">Customer details</h3>
                <InputFormField
                  control={methods.control}
                  label="Name"
                  name="name"
                  layout="horizontal"
                />
                <InputFormField
                  control={methods.control}
                  label="Alias"
                  name="alias"
                  layout="horizontal"
                />
                <InputFormField
                  control={methods.control}
                  label="Email"
                  name="email"
                  layout="horizontal"
                  type="email"
                />
                <InputFormField
                  control={methods.control}
                  label="Invoicing email"
                  name="invoicingEmail"
                  layout="horizontal"
                  type="invoicingEmail"
                />
                <InputFormField
                  control={methods.control}
                  label="Phone"
                  name="phone"
                  layout="horizontal"
                  type="tel"
                />
              </div>

              {/* Tax Information Section */}
              <div className="space-y-4">
                <h3 className="font-semibold">Tax information</h3>
                <InputFormField
                  control={methods.control}
                  label="VAT number"
                  name="vatNumber"
                  layout="horizontal"
                />

                <div className="space-y-2">
                  <Flex align="center" justify="between">
                    <FormLabel>Custom Taxes</FormLabel>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => append({ taxCode: '', name: '', rate: 0 })}
                    >
                      <Plus size={14} className="mr-1" />
                      Add Tax
                    </Button>
                  </Flex>
                  {fields.length > 1 && (
                    <span className="text-xs text-muted-foreground">
                      All taxes will be applied cumulatively to this customer&apos;s invoices
                    </span>
                  )}
                  {fields.map((field, index) => (
                    <Flex key={field.id} className="gap-2 items-start px-1">
                      <FormField
                        control={methods.control}
                        name={`customTaxes.${index}.taxCode`}
                        render={({ field }) => (
                          <FormItem className="flex-1">
                            {index === 0 && <FormLabel>Code</FormLabel>}
                            <FormControl>
                              <Input {...field} placeholder="GST" />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                      <FormField
                        control={methods.control}
                        name={`customTaxes.${index}.name`}
                        render={({ field }) => (
                          <FormItem className="flex-[2]">
                            {index === 0 && <FormLabel>Name</FormLabel>}
                            <FormControl>
                              <Input {...field} placeholder="Goods and Services Tax" />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                      <FormField
                        control={methods.control}
                        name={`customTaxes.${index}.rate`}
                        render={({ field }) => (
                          <FormItem className="flex-1">
                            {index === 0 && <FormLabel>Rate (%)</FormLabel>}
                            <FormControl>
                              <Input
                                {...field}
                                type="number"
                                step="0.01"
                                placeholder="5.0"
                                onChange={e => field.onChange(parseFloat(e.target.value))}
                              />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        onClick={() => remove(index)}
                        className={index === 0 ? 'mt-8' : ''}
                      >
                        <X size={16} />
                      </Button>
                    </Flex>
                  ))}
                </div>

                {!hasCustomTaxes && (
                  <CheckboxFormField
                    control={methods.control}
                    label="Tax exempt"
                    name="isTaxExempt"
                    // layout="horizontal"
                  />
                )}
              </div>

              {/* Billing Address Section */}
              <div className="space-y-4">
                <h3 className="font-semibold">Billing address</h3>
                <InputFormField
                  control={methods.control}
                  label="Address line 1"
                  name="billingAddress.line1"
                  layout="horizontal"
                />
                <InputFormField
                  control={methods.control}
                  label="Address line 2"
                  name="billingAddress.line2"
                  layout="horizontal"
                />
                <InputFormField
                  control={methods.control}
                  label="City"
                  name="billingAddress.city"
                  layout="horizontal"
                />
                <InputFormField
                  control={methods.control}
                  label="State/Province"
                  name="billingAddress.state"
                  layout="horizontal"
                />
                <InputFormField
                  control={methods.control}
                  label="Postal code"
                  name="billingAddress.zipCode"
                  layout="horizontal"
                />
                <CountrySelect
                  control={methods.control}
                  label="Country"
                  name="billingAddress.country"
                  className="col-span-8 bg-input text-muted-foreground"
                  layout="horizontal"
                />
              </div>

              {/* Shipping Address Section */}
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <h3 className="font-semibold">Shipping address</h3>
                  {!showShippingAddress && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() => setShowShippingAddress(true)}
                      className="text-muted-foreground"
                    >
                      <Plus size={16} className="mr-1" />
                      Add shipping address
                    </Button>
                  )}
                </div>

                {showShippingAddress && (
                  <>
                    <CheckboxFormField
                      control={methods.control}
                      label="Same as billing address"
                      name="shippingAddress.sameAsBilling"
                    />

                    {!methods.watch('shippingAddress.sameAsBilling') && (
                      <>
                        <InputFormField
                          control={methods.control}
                          label="Address line 1"
                          name="shippingAddress.address.line1"
                          layout="horizontal"
                        />
                        <InputFormField
                          control={methods.control}
                          label="Address line 2"
                          name="shippingAddress.address.line2"
                          layout="horizontal"
                        />
                        <InputFormField
                          control={methods.control}
                          label="City"
                          name="shippingAddress.address.city"
                          layout="horizontal"
                        />
                        <InputFormField
                          control={methods.control}
                          label="State/Province"
                          name="shippingAddress.address.state"
                          layout="horizontal"
                        />
                        <InputFormField
                          control={methods.control}
                          label="Postal code"
                          name="shippingAddress.address.zipCode"
                          layout="horizontal"
                        />
                        <CountrySelect
                          control={methods.control}
                          label="Country"
                          name="shippingAddress.address.country"
                          className="col-span-8 bg-input text-muted-foreground"
                          layout="horizontal"
                        />
                      </>
                    )}

                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setShowShippingAddress(false)
                        methods.setValue('shippingAddress', undefined)
                      }}
                      className="text-muted-foreground"
                    >
                      <Minus size={16} className="mr-1" />
                      Remove shipping address
                    </Button>
                  </>
                )}
              </div>
            </div>
          </form>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
