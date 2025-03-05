import { Button, Card, ComboboxFormField, Form, InputFormField, Label } from '@md/ui'
import { Edit2, PlusIcon, XIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'

import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useQuery } from '@/lib/connectrpc'
import { Address, Customer } from '@/rpc/api/customers/v1/models_pb'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  getSubscriptionCheckout,
  updateCustomer,
} from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'
import { useQueryClient } from '@tanstack/react-query'

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

export const BillingInfo = ({ customer }: { customer: Customer }) => {
  const [isEditing, setIsEditing] = useState(false)
  const [showTaxNumber, setShowTaxNumber] = useState(!!customer.vatNumber)
  const queryClient = useQueryClient()

  console.log('customer', customer)

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

  const getCountriesQuery = useQuery(getCountries)
  const countries = getCountriesQuery.data?.countries || []

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
        id: customer.id,
        billingAddress: updatedAddress,
        name: values.name,
        vatNumber: showTaxNumber ? values.vatNumber : undefined,
      },
    })
  }

  const countryCode = methods.watch('country')
  const selectedCountry = countries.find(c => c.code === countryCode)

  if (!isEditing) {
    return (
      <>
        <div className="text-sm font-medium">Billing information</div>
        <Card className="mb-8 px-6 py-4 mt-2 border-0" variant="accent">
          <div className="flex justify-between items-start mb-2">
            <div className="text-sm space-y-1">
              <div className="font-medium">{customer.name}</div>
              {customer.billingEmail && (
                <div className="text-muted-foreground">{customer.billingEmail}</div>
              )}
              {customer.billingAddress && (
                <div className="pt-1">
                  {customer.billingAddress.line1}
                  {customer.billingAddress.line2 && <span>, {customer.billingAddress.line2}</span>}
                  <br />
                  {customer.billingAddress.city}, {customer.billingAddress.state}{' '}
                  {customer.billingAddress.zipCode}
                  <br />
                  {selectedCountry ? (
                    <span>
                      {getCountryFlagEmoji(selectedCountry.code)} {selectedCountry.name}
                    </span>
                  ) : (
                    customer.billingAddress.country
                  )}
                </div>
              )}
              {customer.vatNumber && (
                <div className="pt-1">
                  <span className="text-muted-foreground">Tax ID: </span>
                  {customer.vatNumber}
                </div>
              )}
            </div>
            <Button
              variant="ghost"
              size="sm"
              className="text-blue-600 p-0 h-auto"
              onClick={handleEdit}
            >
              <Edit2 size={16} />
            </Button>
          </div>
        </Card>
      </>
    )
  }

  return (
    <div className=" ">
      <Form {...methods}>
        <form
          onSubmit={methods.handleSubmit(onSubmit, err => console.log(err))}
          className="space-y-4 text-xs font-normal"
        >
          <div className="flex justify-between items-center mb-2">
            <div className="text-sm font-medium">Billing information</div>
          </div>

          <InputFormField
            name="billingEmail"
            label="Email"
            disabled
            control={methods.control}
            placeholder="billing@example.com"
            labelClassName="font-normal text-xs"
            className="space-y-1 text-xs"
          />

          <InputFormField
            name="name"
            label="Legal name"
            control={methods.control}
            placeholder="Acme Corp."
            labelClassName="font-normal text-xs"
            className="space-y-1 text-xs"
          />

          <div className="">
            <Label className="font-normal text-xs mb-3">Billing address</Label>

            <ComboboxFormField
              name="country"
              control={methods.control}
              className="rounded-b-none border-b-0 mt-1 text-xs"
              placeholder="Country"
              hasSearch
              options={
                getCountriesQuery.data?.countries.map(country => ({
                  label: (
                    <span className="flex flex-row">
                      <span className="pr-2">{getCountryFlagEmoji(country.code)}</span>
                      <span>{country.name}</span>
                    </span>
                  ),
                  value: country.code,
                  keywords: [country.name, country.code],
                })) ?? []
              }
            />

            <InputFormField
              name="line1"
              control={methods.control}
              placeholder="Address line 1"
              className="rounded-none border-b-0 text-xs"
            />

            <InputFormField
              name="line2"
              control={methods.control}
              placeholder="Apt, suite, etc. (optional)"
              className="rounded-none border-b-0 text-xs"
            />
            <div className="grid grid-cols-2">
              <InputFormField
                name="zipCode"
                control={methods.control}
                placeholder="Postal code"
                className="rounded-none rounded-bl-md border-r-0 text-xs"
              />
              <InputFormField
                name="city"
                control={methods.control}
                placeholder="City"
                className="rounded-none rounded-br-md text-xs"
              />
            </div>
          </div>

          {showTaxNumber ? (
            <div className="relative">
              <InputFormField
                name="vatNumber"
                labelClassName="font-normal text-xs"
                label="Tax number"
                className="text-xs"
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

          <div className="flex justify-end space-x-2 pt-4">
            <Button type="button" variant="ghost" size="sm" onClick={handleCancel}>
              Cancel
            </Button>
            <Button
              type="submit"
              size="sm"
              className="bg-blue-600 hover:bg-blue-700"
              disabled={
                !methods.formState.isDirty ||
                methods.formState.isSubmitting ||
                updateBillingInfoMut.isLoading
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
