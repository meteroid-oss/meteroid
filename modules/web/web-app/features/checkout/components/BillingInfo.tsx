import { create } from '@bufbuild/protobuf'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField, Label } from '@md/ui'
import { QueryKey, useQueryClient } from '@tanstack/react-query'
import { Edit2, PlusIcon, XIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import { refineCompleteBillingInformation } from '@/features/customers/components/BillingInfoForm'
import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { individualName } from '@/lib/schemas/customers'
import { AddressSchema, Customer, CustomerType } from '@/rpc/api/customers/v1/models_pb'
import { getCheckout } from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'
import { getCustomerPortalOverview } from '@/rpc/portal/customer/v1/customer-PortalCustomerService_connectquery'
import { getInvoicePayment } from '@/rpc/portal/invoice/v1/invoice-PortalInvoiceService_connectquery'
import { updateCustomer } from '@/rpc/portal/shared/v1/shared-PortalSharedService_connectquery'

const billingInfoSchema = z.object({
  name: z.string().optional(),
  billingEmail: z.string().trim().email().or(z.literal('')).optional(),
  firstName: z.string().trim().optional(),
  lastName: z.string().trim().optional(),
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
  /** The invoicing entity requires complete billing information (checkout). */
  required?: boolean
  /** A page that holds its own copy of the data (the checkout, re-priced from the address). */
  onUpdated?: (customer: Customer) => void
}

export const BillingInfo = ({
  customer,
  isEditing,
  setIsEditing,
  required = false,
  onUpdated,
}: BillingInfoProps) => {
  const queryClient = useQueryClient()
  const [showTaxNumber, setShowTaxNumber] = useState(!!customer.vatNumber)
  const isIndividual = customer.customerType === CustomerType.INDIVIDUAL

  const updateBillingInfoMut = useMutation(updateCustomer, {
    onSuccess: res => {
      // The same card serves the portal, the checkout and the invoice page: each shows the
      // customer, and the checkout re-prices tax from the new address.
      if (res.customer) {
        const invalidate = (queryKey: QueryKey) => queryClient.invalidateQueries({ queryKey })
        invalidate(
          createConnectQueryKey({ schema: getCustomerPortalOverview, cardinality: undefined })
        )
        invalidate(createConnectQueryKey({ schema: getCheckout, cardinality: undefined }))
        invalidate(createConnectQueryKey({ schema: getInvoicePayment, cardinality: undefined }))
        onUpdated?.(res.customer)
      }

      toast.success('Billing information updated successfully')
      setIsEditing(false)
    },
    onError: () => {
      toast.error('Failed to update billing information')
    },
  })

  const methods = useZodForm({
    schema: billingInfoSchema.superRefine((values, ctx) => {
      if (required) refineCompleteBillingInformation(values, ctx)
      if (!isIndividual) return
      for (const [field, message] of [
        ['firstName', 'First name is required'],
        ['lastName', 'Last name is required'],
        ['country', 'Billing country is required'],
      ] as const) {
        if (!values[field]?.trim()) ctx.addIssue({ code: 'custom', path: [field], message })
      }
    }),
    defaultValues: {
      name: customer.name || '',
      billingEmail: customer.billingEmail || '',
      firstName: customer.firstName || '',
      lastName: customer.lastName || '',
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
    const updatedAddress = create(AddressSchema, {
      line1: values.line1,
      line2: values.line2,
      city: values.city,
      zipCode: values.zipCode,
      country: values.country,
    })

    await updateBillingInfoMut.mutateAsync({
      customer: {
        billingAddress: updatedAddress,
        // An existing email stays as it is; a missing one can be provided here.
        billingEmail: customer.billingEmail ? undefined : values.billingEmail || undefined,
        name: isIndividual ? individualName(values.firstName, values.lastName) : values.name,
        vatNumber: showTaxNumber ? values.vatNumber : '',
        firstName: isIndividual ? values.firstName : undefined,
        lastName: isIndividual ? values.lastName : undefined,
      },
    })
  }

  if (!isEditing) {
    return (
      <div>
        <div className="text-sm font-medium mb-2" style={{ color: 'var(--mtp-text)' }}>
          Billing information
        </div>
        <div
          className="p-4"
          style={{
            border: '1px solid var(--mtp-border)',
            borderRadius: 'var(--mtp-r-ctrl)',
            background: 'var(--mtp-surface)',
          }}
        >
          <div className="flex justify-between items-start">
            <div className="text-sm space-y-1" style={{ color: 'var(--mtp-text)' }}>
              <div className="font-medium">{customer.name}</div>
              {customer.billingEmail && (
                <div style={{ color: 'var(--mtp-text-2)' }}>{customer.billingEmail}</div>
              )}
              {customer.billingAddress && (
                <div className="pt-0">
                  {customer.billingAddress.line1}
                  {customer.billingAddress.line2 && <span>, {customer.billingAddress.line2}</span>}
                  {customer.billingAddress.line1 && <br />}
                  {customer.billingAddress.city}
                  {customer.billingAddress.state && (
                    <span>, {customer.billingAddress.state}</span>
                  )}{' '}
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
                  <span style={{ color: 'var(--mtp-text-2)' }}>Tax ID: </span>
                  {customer.vatNumber}
                </div>
              )}
            </div>
            <button
              onClick={handleEdit}
              className="p-0 h-auto cursor-pointer hover:opacity-80 transition-opacity"
              style={{ color: 'var(--mtp-text-2)' }}
            >
              <Edit2 size={16} />
            </button>
          </div>
        </div>
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
            required={required}
            disabled={!!customer.billingEmail}
            control={methods.control}
            placeholder="billing@example.com"
            labelClassName="font-normal text-xs"
            className="space-y-1 text-xs  focus-visible:shadow-none focus-visible:border-unset"
          />

          {!isIndividual && (
            <InputFormField
              name="name"
              label="Legal name"
              control={methods.control}
              placeholder="Acme Corp."
              labelClassName="font-normal text-xs"
              className="space-y-1 text-xs focus-visible:shadow-none focus-visible:border-unset"
            />
          )}

          {isIndividual && (
            <>
              <div className="grid grid-cols-2 gap-2">
                <InputFormField
                  required
                  name="firstName"
                  label="First name"
                  control={methods.control}
                  placeholder="Jane"
                  labelClassName="font-normal text-xs"
                  className="space-y-1 text-xs  focus-visible:shadow-none focus-visible:border-unset"
                />
                <InputFormField
                  required
                  name="lastName"
                  label="Last name"
                  control={methods.control}
                  placeholder="Doe"
                  labelClassName="font-normal text-xs"
                  className="space-y-1 text-xs  focus-visible:shadow-none focus-visible:border-unset"
                />
              </div>
            </>
          )}

          <div className="">
            <Label className="font-normal text-xs mb-3">
              Billing address{isIndividual && !required && ' (country required)'}
              {required && <span className="text-destructive text-xs pl-1">*</span>}
            </Label>

            <CountrySelect
              required={isIndividual || required}
              name="country"
              control={methods.control}
              className="rounded-b-none border-b-0 mt-1 text-xs focus:relative focus:z-10"
              placeholder="Country"
              label=""
            />

            <InputFormField
              name="line1"
              control={methods.control}
              placeholder="Address line 1"
              className="rounded-none border-b-0 text-xs focus-visible:relative focus-visible:z-10"
            />

            <InputFormField
              name="line2"
              control={methods.control}
              placeholder="Apt, suite, etc. (optional)"
              className="rounded-none border-b-0 text-xs focus-visible:relative focus-visible:z-10"
            />
            <div className="grid grid-cols-2">
              <InputFormField
                name="zipCode"
                control={methods.control}
                placeholder="Postal code"
                className="rounded-none rounded-bl-md border-r-0 text-xs focus-visible:relative focus-visible:z-10"
              />
              <InputFormField
                name="city"
                control={methods.control}
                placeholder="City"
                className="rounded-none rounded-br-md text-xs focus-visible:relative focus-visible:z-10"
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
          ) : !isIndividual ? (
            <Button
              type="button"
              variant="ghost"
              className="text-xs flex items-center text-blue-600 h-8 px-0"
              onClick={() => setShowTaxNumber(true)}
            >
              <PlusIcon size={16} className="mr-1" /> Add tax number
            </Button>
          ) : null}

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
