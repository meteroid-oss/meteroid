import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Form, Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ComponentProps, useEffect } from 'react'
import { toast } from 'sonner'

import { CustomerFormFields } from '@/features/customers/form/CustomerFormFields'
import { useZodForm } from '@/hooks/useZodForm'
import { editCustomerSchema, EditCustomerSchema } from '@/lib/schemas/customers'
import {
  getCustomerById,
  listCustomers,
  updateCustomer,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof Modal>, 'visible' | 'onCancel'> & {
  customer: Customer
}

export const EditCustomerModal = ({ customer, visible, onCancel }: Props) => {
  const queryClient = useQueryClient()

  const updateCustomerMutation = useMutation(updateCustomer, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getCustomerById, { id: customer.id }),
      })

      queryClient.invalidateQueries({
        queryKey: [listCustomers.service.typeName],
      })
    },
  })

  const getDefaultValues = (): EditCustomerSchema => ({
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
    schema: editCustomerSchema,
    defaultValues: getDefaultValues(),
  })

  useEffect(() => {
    if (visible) {
      methods.reset(getDefaultValues())
    }
  }, [visible, customer])

  const onSubmit = async (data: EditCustomerSchema) => {
    await updateCustomerMutation.mutateAsync({
      customer: {
        id: customer.id,
        name: data.name,
        alias: data.alias,
        billingEmail: data.email,
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

  const hasShippingAddress = Boolean(customer.shippingAddress?.address)

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
              <CustomerFormFields
                control={methods.control}
                initialShowShippingAddress={hasShippingAddress}
              />
            </div>
          </form>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
