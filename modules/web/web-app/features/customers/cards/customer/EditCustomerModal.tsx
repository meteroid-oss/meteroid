import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { Form, FormInput, Modal } from '@ui2/components'
import { ComponentProps } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { customerSchema } from '@/features/customers/cards/customer/schema'
import { useZodForm } from '@/hooks/useZodForm'
import {
  getCustomer,
  patchCustomer,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof Modal>, 'visible' | 'onCancel'> & {
  customer: Customer
}

export const EditCustomerModal = ({ customer, ...props }: Props) => {
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
    schema: customerSchema,
    defaultValues: customer,
  })

  const onSubmit = async (data: z.infer<typeof customerSchema>) => {
    await patchCustomerMutation.mutateAsync({
      customer: {
        id: customer.id,
        name: data.name,
        alias: data.alias,
        email: data.email,
        invoicingEmail: data.invoicingEmail,
        phone: data.phone,
      },
    })
    toast.success('Address updated')
    props.onCancel?.()
  }

  return (
    <Modal
      header={<>Edit customer</>}
      {...props}
      onConfirm={() => methods.handleSubmit(onSubmit)()}
    >
      <Modal.Content>
        <Form {...methods}>
          <form>
            <div className="py-4 w-full space-y-4">
              <h3 className="font-semibold">Customer details</h3>
              <FormInput label="Name" name="name" direction="horizontal" />
              <FormInput label="Alias" name="alias" direction="horizontal" />
              <FormInput label="Email" name="email" direction="horizontal" type="email" />
              <FormInput
                label="Invoicing email"
                name="invoicingEmail"
                direction="horizontal"
                type="email"
              />
              <FormInput label="Phone" name="phone" direction="horizontal" type="tel" />
            </div>
          </form>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
