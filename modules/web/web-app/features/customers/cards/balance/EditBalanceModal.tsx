import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { FormItem, Input, Modal } from '@ui/components'
import { ComponentProps } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { balanceSchema } from '@/features/customers/cards/balance/schema'
import { useZodForm } from '@/hooks/useZodForm'
import {
  getCustomer,
  patchCustomer,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof Modal>, 'visible' | 'onCancel'> & {
  customer: Customer
}

export const EditBalanceModal = ({ customer, ...props }: Props) => {
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
    schema: balanceSchema,
    defaultValues: {
      balanceValueCents: customer.balanceValueCents,
      balanceCurrency: customer.balanceCurrency as z.infer<typeof balanceSchema>['balanceCurrency'],
    },
  })

  const onSubmit = async (data: z.infer<typeof balanceSchema>) => {
    await patchCustomerMutation.mutateAsync({
      customer: {
        id: customer.id,
        balanceValueCents: data.balanceValueCents,
        balanceCurrency: data.balanceCurrency,
      },
    })
    toast.success('Balance updated')
    props.onCancel?.()
  }

  return (
    <Modal
      size="small"
      header={<>Edit address</>}
      {...props}
      onConfirm={() => methods.handleSubmit(onSubmit)()}
    >
      <Modal.Content>
        <form>
          <div className="py-4 w-full space-y-4">
            <FormItem label="Balance" {...methods.withError('balanceValueCents')}>
              <Input
                className="max-w-xs"
                type="number"
                {...methods.register('balanceValueCents', {
                  valueAsNumber: true,
                })}
              />
            </FormItem>
            <FormItem label="Currency" {...methods.withError('balanceCurrency')}>
              <Input className="max-w-xs" {...methods.register('balanceCurrency')} />
            </FormItem>
          </div>
        </form>
      </Modal.Content>
    </Modal>
  )
}
