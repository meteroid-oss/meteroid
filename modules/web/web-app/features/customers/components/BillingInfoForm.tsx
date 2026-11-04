import { Button, Form, InputFormField, Label } from '@md/ui'
import { PlusIcon, XIcon } from 'lucide-react'
import { useState } from 'react'
import { UseFormReturn, useFormContext } from 'react-hook-form'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import { CustomerSchema } from '@/rpc/api/customers/v1/models_pb'

import type { MessageInitShape } from '@bufbuild/protobuf'

export const billingInfoSchema = z.object({
  name: z.string().optional(),
  billingEmail: z.string().email().optional(),
  line1: z.string().optional(),
  line2: z.string().optional(),
  city: z.string().optional(),
  state: z.string().optional(),
  zipCode: z.string().optional(),
  country: z.string().optional(),
  vatNumber: z.string().optional(),
  legalNumber: z.string().optional(),
})

export type BillingInfoFormValues = z.infer<typeof billingInfoSchema>

const REQUIRED_BILLING_FIELDS = [
  ['billingEmail', 'Billing email is required'],
  ['line1', 'Address line 1 is required'],
  ['zipCode', 'Postal code is required'],
  ['city', 'City is required'],
  ['country', 'Country is required'],
] as const

// What an invoicing entity requiring billing information asks for, as the server checks it
// (`Customer::has_complete_billing_information`).
export const refineCompleteBillingInformation = (
  values: Pick<BillingInfoFormValues, (typeof REQUIRED_BILLING_FIELDS)[number][0]>,
  ctx: z.RefinementCtx
) => {
  for (const [field, message] of REQUIRED_BILLING_FIELDS) {
    if (!values[field]?.trim()) ctx.addIssue({ code: 'custom', path: [field], message })
  }
}

export type BillingInfoFieldValues = BillingInfoFormValues & {
  firstName?: string
  lastName?: string
}

interface BillingInfoFieldsProps {
  /** First and last name instead of a legal name; no tax or registration number. */
  individual?: boolean
  emailDisabled?: boolean
  /** `country`: the country alone. */
  address?: 'optional' | 'required' | 'country'
  /** `required`: always shown, and can't be dismissed. */
  taxNumber?: 'hidden' | 'optional' | 'required'
  collectLegalNumber?: boolean
  initialShowTaxNumber?: boolean
  /** Browser autofill, for forms buyers fill in about themselves. */
  autofill?: boolean
  onBlur?: () => void
}

/** The billing fields alone, for a form whose values extend `BillingInfoFieldValues`. */
export const BillingInfoFields = ({
  individual = false,
  emailDisabled = false,
  address = 'optional',
  taxNumber = 'optional',
  collectLegalNumber = true,
  initialShowTaxNumber = false,
  autofill = false,
  onBlur,
}: BillingInfoFieldsProps) => {
  const { control, setValue } = useFormContext<BillingInfoFieldValues>()
  const [showTaxNumber, setShowTaxNumber] = useState(initialShowTaxNumber)
  const auto = (token: string) => (autofill ? token : undefined)
  const fieldClass = 'text-xs focus-visible:shadow-none'
  const hideTaxNumber = () => {
    setShowTaxNumber(false)
    setValue('vatNumber', '', { shouldDirty: true })
    onBlur?.()
  }

  return (
    <>
      <InputFormField
        name="billingEmail"
        label="Email"
        type="email"
        autoComplete={auto('email')}
        disabled={emailDisabled}
        control={control}
        placeholder="billing@example.com"
        labelClassName="font-normal text-xs"
        className="space-y-1 text-xs"
      />

      {individual ? (
        <div className="grid grid-cols-2 gap-2">
          <InputFormField
            required
            name="firstName"
            label="First name"
            autoComplete={auto('given-name')}
            control={control}
            placeholder="Jane"
            labelClassName="font-normal text-xs"
            className="space-y-1 text-xs"
            onBlur={onBlur}
          />
          <InputFormField
            required
            name="lastName"
            label="Last name"
            autoComplete={auto('family-name')}
            control={control}
            placeholder="Doe"
            labelClassName="font-normal text-xs"
            className="space-y-1 text-xs"
            onBlur={onBlur}
          />
        </div>
      ) : (
        <InputFormField
          name="name"
          label="Legal name"
          autoComplete={auto('organization')}
          control={control}
          placeholder="Acme Corp."
          labelClassName="font-normal text-xs"
          className="space-y-1 text-xs"
          onBlur={onBlur}
        />
      )}

      {address === 'country' ? (
        <CountrySelect
          required
          name="country"
          control={control}
          label="Country"
          labelClassName="font-normal text-xs"
          className="text-xs"
          placeholder="Select a country"
        />
      ) : (
        <div className="">
          <Label className="font-normal text-xs mb-3">
            Billing address
            {address === 'required' && <span className="text-destructive text-xs pl-1">*</span>}
          </Label>

          <CountrySelect
            name="country"
            control={control}
            className="rounded-b-none border-b-0 mt-1 text-xs "
            placeholder="Country"
            label=""
          />

          <InputFormField
            name="line1"
            autoComplete={auto('address-line1')}
            control={control}
            placeholder="Address line 1"
            className={`rounded-none border-b-0 ${fieldClass}`}
            onBlur={onBlur}
          />

          <InputFormField
            name="line2"
            autoComplete={auto('address-line2')}
            control={control}
            placeholder="Apt, suite, etc. (optional)"
            className={`rounded-none border-b-0 ${fieldClass}`}
            onBlur={onBlur}
          />
          <div className="grid grid-cols-3">
            <InputFormField
              name="zipCode"
              autoComplete={auto('postal-code')}
              control={control}
              placeholder="Postal code"
              className={`rounded-none rounded-bl-md border-r-0 ${fieldClass}`}
              onBlur={onBlur}
            />
            <InputFormField
              name="city"
              autoComplete={auto('address-level2')}
              control={control}
              placeholder="City"
              className={`rounded-none border-r-0 ${fieldClass}`}
              onBlur={onBlur}
            />
            <InputFormField
              name="state"
              autoComplete={auto('address-level1')}
              control={control}
              placeholder="State"
              className={`rounded-none rounded-br-md ${fieldClass}`}
              onBlur={onBlur}
            />
          </div>
        </div>
      )}

      {!individual && collectLegalNumber && (
        <InputFormField
          name="legalNumber"
          labelClassName="font-normal text-xs "
          label="Registration number"
          className={fieldClass}
          control={control}
          placeholder="SIREN / SIRET"
          onBlur={onBlur}
        />
      )}

      {!individual &&
        taxNumber !== 'hidden' &&
        (showTaxNumber || taxNumber === 'required' ? (
          <div className="relative">
            <InputFormField
              name="vatNumber"
              labelClassName="font-normal text-xs "
              label="Tax number"
              className={fieldClass}
              control={control}
              placeholder="FR12345678900"
              onBlur={onBlur}
            />
            {taxNumber === 'optional' && (
              <Button
                type="button"
                size="icon"
                variant="ghost"
                className="absolute top-0 right-0 h-5 w-5"
                onClick={hideTaxNumber}
              >
                <XIcon size={14} />
              </Button>
            )}
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
        ))}
    </>
  )
}

interface BillingInfoFormProps {
  customer: MessageInitShape<typeof CustomerSchema>
  methods: UseFormReturn<BillingInfoFormValues>
  onSubmit: (values: BillingInfoFormValues) => Promise<void>
  onBlur?: (values: BillingInfoFormValues) => Promise<void>
  onCancel: () => void
  isSubmitting?: boolean
  title?: string
  hideActions?: boolean // Hide Save/Cancel buttons when used in a parent form context
  allowEmailEdit?: boolean // Allow editing email when not already set (for checkout flow)
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
  allowEmailEdit = false,
}: BillingInfoFormProps) => {
  return (
    <div className="">
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4 text-xs font-normal">
          {title && (
            <div className="flex justify-between items-center mb-2">
              <div className="text-sm font-medium">{title}</div>
            </div>
          )}

          <BillingInfoFields
            emailDisabled={!allowEmailEdit || !!customer.billingEmail}
            initialShowTaxNumber={!!customer.vatNumber}
            onBlur={() => onBlurAll?.(methods.getValues())}
          />

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
