import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import { useZodForm } from '@/hooks/useZodForm'
import { Address, Customer } from '@/rpc/api/customers/v1/models_pb'
import {
  getSubscriptionCheckout,
  updateCustomer,
} from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'

import { BillingInfoCard } from './BillingInfoCard'
import { BillingInfoForm, BillingInfoFormValues, billingInfoSchema } from './BillingInfoForm'

interface BillingInfoProps {
  customer: Customer
  isEditing: boolean
  setIsEditing: (isEditing: boolean) => void
}

export const BillingInfo = ({ customer, isEditing, setIsEditing }: BillingInfoProps) => {
  const queryClient = useQueryClient()

  const updateBillingInfoMut = useMutation(updateCustomer, {
    onSuccess: res => {
      if (res.customer) {
        queryClient.setQueryData(
          createConnectQueryKey(getSubscriptionCheckout),
          createProtobufSafeUpdater(getSubscriptionCheckout, prev => ({
            checkout: {
              ...prev?.checkout,
              customer: res.customer,
            },
          }))
        )
      }

      toast.success('Billing information updated successfully')
      setIsEditing(false)
    },
    onError: () => {
      toast.error('Failed to update billing information')
    },
  })

  const methods = useZodForm({
    schema: billingInfoSchema,
    defaultValues: {
      name: customer.name || '',
      billingEmail: customer.billingEmail || '',
      line1: customer.billingAddress?.line1 || '',
      line2: customer.billingAddress?.line2 || '',
      city: customer.billingAddress?.city || '',
      zipCode: customer.billingAddress?.zipCode || '',
      country: customer.billingAddress?.country,
      vatNumber: customer.vatNumber || '',
    },
    mode: 'onSubmit',
    reValidateMode: 'onSubmit',
  })

  const handleEdit = () => {
    setIsEditing(true)
  }

  const handleCancel = () => {
    methods.reset()
    setIsEditing(false)
  }

  const onSubmit = async (values: BillingInfoFormValues) => {
    // Create new Address object
    const updatedAddress = new Address({
      line1: values.line1,
      line2: values.line2,
      city: values.city,
      zipCode: values.zipCode,
      country: values.country,
    })

    await updateBillingInfoMut.mutateAsync({
      customer: {
        id: customer.id,
        billingAddress: updatedAddress,
        name: values.name,
        vatNumber: values.vatNumber,
      },
    })
  }

  if (!isEditing) {
    return <BillingInfoCard customer={customer} onEdit={handleEdit} />
  }

  return (
    <BillingInfoForm
      customer={customer}
      methods={methods}
      onSubmit={onSubmit}
      onCancel={handleCancel}
      isSubmitting={updateBillingInfoMut.isPending}
    />
  )
}
