import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { Form, Modal, CheckboxFormField, InputFormField } from '@md/ui'
import { ComponentProps } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { addressesSchema } from '@/features/customers/cards/address/schema'
import { useZodForm } from '@/hooks/useZodForm'
import {
  getCustomer,
  patchCustomer,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof Modal>, 'visible' | 'onCancel'> & {
  customer: Customer
}

export const EditAddressModal = ({ customer, ...props }: Props) => {
  const queryClient = useQueryClient()
  const patchCustomerMutation = useMutation(patchCustomer, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getCustomer, { id: customer.id }),
      })
    },
  })

  const methods = useZodForm({
    mode: 'onChange',
    schema: addressesSchema,
    defaultValues: {
      billing_address: customer.billingAddress,
      shipping_address: {
        ...customer.shippingAddress,
        sameAsBilling: customer.shippingAddress?.sameAsBilling !== false,
      },
    },
  })
  const sameShippingAddress = methods.watch('shipping_address.sameAsBilling')

  const onSubmit = async (data: z.infer<typeof addressesSchema>) => {
    await patchCustomerMutation.mutateAsync({
      customer: {
        id: customer.id,
        billingAddress: data.billing_address,
        shippingAddress: data.shipping_address,
      },
    })
    toast.success('Address updated')
    props.onCancel?.()
    methods.reset()
  }
  const inputProps = {
    control: methods.control,
    direction: 'horizontal' as const,
  }

  return (
    <Modal header={<>Edit address</>} {...props} onConfirm={() => methods.handleSubmit(onSubmit)()}>
      <Modal.Content>
        <Form {...methods}>
          <form>
            <div className="py-4 w-full space-y-4">
              <h3 className="font-semibold">Billing address</h3>
              <InputFormField label="Line 1" name="billing_address.line1" {...inputProps} />
              <InputFormField label="Line 2" name="billing_address.line2" {...inputProps} />
              <InputFormField label="City" name="billing_address.city" {...inputProps} />
              <InputFormField label="Country" name="billing_address.country" {...inputProps} />
              <InputFormField label="State" name="billing_address.state" {...inputProps} />
              <InputFormField label="Zip Code" name="billing_address.zipcode" {...inputProps} />

              <h3 className="font-semibold">Shipping address</h3>
              <CheckboxFormField
                name="shipping_address.sameAsBilling"
                label="Same as billing address"
                variant="card"
                control={methods.control}
              />

              {sameShippingAddress || (
                <>
                  <InputFormField
                    label="Line 1"
                    name="shipping_address.address.line1"
                    {...inputProps}
                  />
                  <InputFormField
                    label="Line 2"
                    name="shipping_address.address.line2"
                    {...inputProps}
                  />
                  <InputFormField
                    label="City"
                    name="shipping_address.address.city"
                    {...inputProps}
                  />
                  <InputFormField
                    label="Country"
                    name="shipping_address.address.country"
                    {...inputProps}
                  />
                  <InputFormField
                    label="State"
                    name="shipping_address.address.state"
                    {...inputProps}
                  />
                  <InputFormField
                    label="Zip Code"
                    name="shipping_address.address.zipcode"
                    {...inputProps}
                  />
                </>
              )}
            </div>
          </form>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
