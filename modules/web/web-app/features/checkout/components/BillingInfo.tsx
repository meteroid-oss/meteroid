import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button, Form, InputFormField, Label } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Edit2, PlusIcon, XIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { Address, Customer } from '@/rpc/api/customers/v1/models_pb'
import { getCustomerPortalOverview } from '@/rpc/portal/customer/v1/customer-PortalCustomerService_connectquery'
import { updateCustomer } from '@/rpc/portal/shared/v1/shared-PortalSharedService_connectquery'

const billingInfoSchema = z.object({
  name: z.string().optional(),
  billingEmail: z.string().email().optional(),
  line1: z.string().optional(),
  line2: z.string().optional(),
  city: z.string().optional(),
  zipCode: z.string().optional(),
  country: z.string().optional(),
  vatNumber: z.string().optional(),
})

interface BillingInfoProps {
  customer: Customer
  isEditing: boolean
  setIsEditing: (isEditing: boolean) => void
}

export const BillingInfo = ({ customer, isEditing, setIsEditing }: BillingInfoProps) => {
  const queryClient = useQueryClient()
  const [showTaxNumber, setShowTaxNumber] = useState(!!customer.vatNumber)

  const updateBillingInfoMut = useMutation(updateCustomer, {
    onSuccess: res => {
      if (res.customer) {
        queryClient.setQueryData(
          createConnectQueryKey(getCustomerPortalOverview),
          createProtobufSafeUpdater(getCustomerPortalOverview, prev => ({
            overview: {
              ...prev?.overview,
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
    setShowTaxNumber(!!customer.vatNumber)
  }

  const onSubmit = async (values: z.infer<typeof billingInfoSchema>) => {
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
        billingAddress: updatedAddress,
        name: values.name,
        vatNumber: showTaxNumber ? values.vatNumber : undefined,
      },
    })
  }

  if (!isEditing) {
    return (
      <div className="flex justify-between items-start">
        <div className="text-sm space-y-1">
          <div className="font-medium">{customer.name}</div>
          {customer.billingEmail && (
            <div className="text-gray-500">{customer.billingEmail}</div>
          )}
          {customer.billingAddress && (
            <div className="pt-0">
              {customer.billingAddress.line1}
              {customer.billingAddress.line2 && <span>, {customer.billingAddress.line2}</span>}
              {customer.billingAddress.line1 && <br />}
              {customer.billingAddress.city}
              {customer.billingAddress.state && <span>, {customer.billingAddress.state}</span>}{' '}
              {customer.billingAddress.zipCode}
              <br />
              {customer.billingAddress.country && (
                <span>
                  {getCountryFlagEmoji(customer.billingAddress.country)}{' '}
                  {getCountryName(customer.billingAddress.country)}
                </span>
              )}
            </div>
          )}
          {customer.vatNumber && (
            <div className="pt-1">
              <span className="text-gray-500">Tax ID: </span>
              {customer.vatNumber}
            </div>
          )}
        </div>
        <button onClick={handleEdit} className="p-0 h-auto text-gray-600 hover:text-gray-900">
          <Edit2 size={16} />
        </button>
      </div>
    )
  }

  return (
    <div>
      <Form {...methods}>
        <form
          onSubmit={methods.handleSubmit(onSubmit, err => console.log(err))}
          className="space-y-4"
        >
          <div className="mb-4">
            <h3 className="text-base font-medium text-gray-900">Edit billing information</h3>
          </div>

          <InputFormField
            name="billingEmail"
            label="Email"
            disabled={!!customer.billingEmail}
            control={methods.control}
            placeholder="billing@example.com"
            labelClassName="font-normal text-xs"
            className="space-y-1 text-xs  focus-visible:shadow-none focus-visible:border-unset"
          />

          <InputFormField
            name="name"
            label="Legal name"
            control={methods.control}
            placeholder="Acme Corp."
            labelClassName="font-normal text-xs"
            className="space-y-1 text-xs  focus-visible:shadow-none focus-visible:border-unset"
          />

          <div className="">
            <Label className="font-normal text-xs mb-3">Billing address</Label>

            <CountrySelect
              name="country"
              control={methods.control}
              className="rounded-b-none border-b-0 mt-1 text-xs"
              placeholder="Country"
              label=""
            />

            <InputFormField
              name="line1"
              control={methods.control}
              placeholder="Address line 1"
              className="rounded-none border-b-0 text-xs  focus-visible:shadow-none focus-visible:border-unset"
            />

            <InputFormField
              name="line2"
              control={methods.control}
              placeholder="Apt, suite, etc. (optional)"
              className="rounded-none border-b-0 text-xs  focus-visible:shadow-none focus-visible:border-unset"
            />
            <div className="grid grid-cols-2">
              <InputFormField
                name="zipCode"
                control={methods.control}
                placeholder="Postal code"
                className="rounded-none rounded-bl-md border-r-0 text-xs  focus-visible:shadow-none focus-visible:border-unset"
              />
              <InputFormField
                name="city"
                control={methods.control}
                placeholder="City"
                className="rounded-none rounded-br-md text-xs  focus-visible:shadow-none focus-visible:border-unset"
              />
            </div>
          </div>

          {showTaxNumber ? (
            <div className="relative">
              <InputFormField
                name="vatNumber"
                labelClassName="font-normal text-xs"
                label="Tax number"
                className="text-xs  focus-visible:shadow-none focus-visible:border-unset  "
                control={methods.control}
                placeholder="FR12345678900"
              />
              <Button
                type="button"
                size="icon"
                variant="ghost"
                className="absolute top-0 right-0 h-5 w-5"
                onClick={() => setShowTaxNumber(false)}
              >
                <XIcon size={14} />
              </Button>
            </div>
          ) : (
            <Button
              type="button"
              variant="ghost"
              className="text-xs flex items-center text-blue-600 h-8 px-0"
              onClick={() => setShowTaxNumber(true)}
            >
              <PlusIcon size={16} className="mr-1" /> Add tax number
            </Button>
          )}

          <div className="flex justify-end space-x-3 pt-6 border-t border-gray-100">
            <Button type="button" variant="outline" onClick={handleCancel} className="font-medium">
              Cancel
            </Button>
            <Button
              type="submit"
              className="bg-gray-900 hover:bg-gray-800 text-white font-medium"
              disabled={
                !methods.formState.isDirty ||
                methods.formState.isSubmitting ||
                updateBillingInfoMut.isPending
              }
            >
              Save changes
            </Button>
          </div>
        </form>
      </Form>
    </div>
  )
}
