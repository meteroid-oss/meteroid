import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, CheckboxFormField, Form, InputFormField, Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Minus, Plus } from 'lucide-react'
import { ComponentProps, useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

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

export const EditCustomerModal = ({ customer, ...props }: Props) => {
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

  const methods = useZodForm({
    mode: 'onChange',
    schema: customerSchema,
    defaultValues: {
      name: customer.name,
      alias: customer.alias,
      email: customer.billingEmail,
      invoicingEmail: customer.invoicingEmails[0] || '',
      phone: customer.phone,
      vatNumber: customer.vatNumber,
      customTaxRate: customer.customTaxRate,
      isTaxExempt: customer.isTaxExempt,
      billingAddress: customer.billingAddress,
      shippingAddress: customer.shippingAddress,
    },
  })

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
        customTaxRate: data.customTaxRate,
        isTaxExempt: data.isTaxExempt,
        billingAddress: data.billingAddress,
        shippingAddress: data.shippingAddress,
      },
    })
    toast.success('Customer updated')
    props.onCancel?.()
  }

  return (
    <Modal
      header={<>Edit customer</>}
      {...props}
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
                  type="email"
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
                <InputFormField
                  control={methods.control}
                  label="Custom tax rate"
                  name="customTaxRate"
                  layout="horizontal"
                  placeholder="0.20"
                />
                <CheckboxFormField
                  control={methods.control}
                  label="Tax exempt"
                  name="isTaxExempt"
                  // layout="horizontal"
                />
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
                <InputFormField
                  control={methods.control}
                  label="Country"
                  name="billingAddress.country"
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
                        <InputFormField
                          control={methods.control}
                          label="Country"
                          name="shippingAddress.address.country"
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
