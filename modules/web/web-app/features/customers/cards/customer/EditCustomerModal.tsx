import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Form, InputFormField, Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
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
              <InputFormField label="Name" name="name" layout="horizontal" />
              <InputFormField label="Alias" name="alias" layout="horizontal" />
              <InputFormField label="Email" name="email" layout="horizontal" type="email" />
              <InputFormField
                label="Invoicing email"
                name="invoicingEmail"
                layout="horizontal"
                type="email"
              />
              <InputFormField label="Phone" name="phone" layout="horizontal" type="tel" />
            </div>
          </form>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
