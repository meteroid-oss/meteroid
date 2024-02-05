import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { CheckboxFormItem, FormItem, Input, Modal } from '@ui/components'
import { ComponentProps } from 'react'
import { FieldPath } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { ControlledCheckbox } from '@/components/form/ControlledCheckbox'
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
        sameThanBilling: customer.shippingAddress?.sameThanBilling !== false,
      },
    },
  })
  const sameShippingAddress = methods.watch('shipping_address.sameThanBilling')

  const withItem = (label: string, name: FieldPath<z.TypeOf<typeof addressesSchema>>) => {
    return (
      <FormItem label={label} layout="horizontal" {...methods.withError(name)}>
        <Input className="max-w-xs" {...methods.register(name)} />
      </FormItem>
    )
  }

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

  return (
    <Modal header={<>Edit address</>} {...props} onConfirm={() => methods.handleSubmit(onSubmit)()}>
      <Modal.Content>
        <form>
          <div className="py-4 w-full space-y-4">
            <h3 className="font-semibold">Billing address</h3>
            {withItem('Line 1', 'billing_address.line1')}
            {withItem('Line 2', 'billing_address.line2')}
            {withItem('City', 'billing_address.city')}
            {withItem('Country', 'billing_address.country')}
            {withItem('State', 'billing_address.state')}
            {withItem('Zip Code', 'billing_address.zipcode')}

            <h3 className="font-semibold">Shipping address</h3>
            <CheckboxFormItem
              name="shipping_address.sameThanBilling"
              label="Same as billing address"
            >
              <ControlledCheckbox
                {...methods.withControl(`shipping_address.sameThanBilling`)}
                id="shipping_address.sameThanBilling"
              />
            </CheckboxFormItem>

            {sameShippingAddress || (
              <>
                {withItem('Line 1', 'shipping_address.address.line1')}
                {withItem('Line 2', 'shipping_address.address.line2')}
                {withItem('City', 'shipping_address.address.city')}
                {withItem('Country', 'shipping_address.address.country')}
                {withItem('State', 'shipping_address.address.state')}
                {withItem('Zip Code', 'shipping_address.address.zipcode')}
              </>
            )}
          </div>
        </form>
      </Modal.Content>
    </Modal>
  )
}
