import { PartialMessage } from '@bufbuild/protobuf'
import { Button, Form, InputFormField, Label } from '@md/ui'
import { PlusIcon, XIcon } from 'lucide-react'
import { useState } from 'react'
import { UseFormReturn } from 'react-hook-form'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

export const billingInfoSchema = z.object({
  name: z.string().optional(),
  billingEmail: z.string().email().optional(),
  line1: z.string().optional(),
  line2: z.string().optional(),
  city: z.string().optional(),
  zipCode: z.string().optional(),
  country: z.string().optional(),
  vatNumber: z.string().optional(),
})

export type BillingInfoFormValues = z.infer<typeof billingInfoSchema>

interface BillingInfoFormProps {
  customer: PartialMessage<Customer>
  methods: UseFormReturn<BillingInfoFormValues>
  onSubmit: (values: BillingInfoFormValues) => Promise<void>
  onBlur?: (values: BillingInfoFormValues) => Promise<void>
  onCancel: () => void
  isSubmitting?: boolean
  title?: string
  hideActions?: boolean // Hide Save/Cancel buttons when used in a parent form context
}

export const BillingInfoForm = ({
  customer,
  methods,
  onSubmit,
  onBlur: onBlurAll,
  onCancel,
  isSubmitting,
  title = 'Billing information',
  hideActions = false,
}: BillingInfoFormProps) => {
  const [showTaxNumber, setShowTaxNumber] = useState(!!customer.vatNumber)

  const handleBlur = () => {
    onBlurAll?.(methods.getValues())
  }

  return (
    <div className="">
      <Form {...methods}>
        <form
          onSubmit={methods.handleSubmit(onSubmit)}
          className="space-y-4 text-xs font-normal"
        >
          {title && (
            <div className="flex justify-between items-center mb-2">
              <div className="text-sm font-medium">{title}</div>
            </div>
          )}

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
            onBlur={handleBlur}
          />

          <div className="">
            <Label className="font-normal text-xs mb-3">Billing address</Label>

            <CountrySelect
              name="country"
              control={methods.control}
              className="rounded-b-none border-b-0 mt-1 text-xs "
              placeholder="Country"
              label=""
            />

            <InputFormField
              name="line1"
              control={methods.control}
              placeholder="Address line 1"
              className="rounded-none border-b-0 text-xs focus-visible:shadow-none"
              onBlur={handleBlur}
            />

            <InputFormField
              name="line2"
              control={methods.control}
              placeholder="Apt, suite, etc. (optional)"
              className="rounded-none border-b-0 text-xs focus-visible:shadow-none"
              onBlur={handleBlur}
            />
            <div className="grid grid-cols-2">
              <InputFormField
                name="zipCode"
                control={methods.control}
                placeholder="Postal code"
                className="rounded-none rounded-bl-md border-r-0 text-xs focus-visible:shadow-none"
                onBlur={handleBlur}
              />
              <InputFormField
                name="city"
                control={methods.control}
                placeholder="City"
                className="rounded-none rounded-br-md text-xs focus-visible:shadow-none"
                onBlur={handleBlur}
              />
            </div>
          </div>

          {showTaxNumber ? (
            <div className="relative">
              <InputFormField
                name="vatNumber"
                labelClassName="font-normal text-xs "
                label="Tax number"
                className="text-xs focus-visible:shadow-none"
                control={methods.control}
                placeholder="FR12345678900"
                onBlur={handleBlur}
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

          {!hideActions && (
            <div className="flex justify-end space-x-2 pt-4">
              <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
                Cancel
              </Button>
              <Button
                type="submit"
                size="sm"
                className="bg-blue-600 hover:bg-blue-700"
                disabled={
                  !methods.formState.isDirty || methods.formState.isSubmitting || isSubmitting
                }
              >
                Save changes
              </Button>
            </div>
          )}
        </form>
      </Form>
    </div>
  )
}
