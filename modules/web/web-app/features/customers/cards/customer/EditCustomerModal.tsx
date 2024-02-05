import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { FormItem, Input, Modal } from '@ui/components'
import { ComponentProps } from 'react'
import { FieldPath } from 'react-hook-form'
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

  const withItem = (label: string, name: FieldPath<z.TypeOf<typeof customerSchema>>) => {
    return (
      <FormItem label={label} layout="horizontal" {...methods.withError(name)}>
        <Input className="max-w-xs" {...methods.register(name)} />
      </FormItem>
    )
  }

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
        <form>
          <div className="py-4 w-full space-y-4">
            <h3 className="font-semibold">Billing address</h3>
            {withItem('Name', 'name')}
            {withItem('Alias', 'alias')}
            {withItem('Email', 'email')}
            {withItem('Invoicing email', 'invoicingEmail')}
            {withItem('Phone', 'phone')}
          </div>
        </form>
      </Modal.Content>
    </Modal>
  )
}
