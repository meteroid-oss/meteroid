import { z } from 'zod'

import { CustomerTaxStatus } from '@/rpc/api/customers/v1/models_pb'

export type TaxStatus = 'TAXABLE' | 'EXEMPT' | 'REVERSE_CHARGE'

export const taxStatusToProto = (status: TaxStatus): CustomerTaxStatus => {
  switch (status) {
    case 'EXEMPT':
      return CustomerTaxStatus.EXEMPT
    case 'REVERSE_CHARGE':
      return CustomerTaxStatus.REVERSE_CHARGE
    case 'TAXABLE':
    default:
      return CustomerTaxStatus.TAXABLE
  }
}

export const taxStatusFromProto = (status: CustomerTaxStatus | undefined): TaxStatus => {
  switch (status) {
    case CustomerTaxStatus.EXEMPT:
      return 'EXEMPT'
    case CustomerTaxStatus.REVERSE_CHARGE:
      return 'REVERSE_CHARGE'
    default:
      return 'TAXABLE'
  }
}

const addressSchema = z.object({
  line1: z.string().optional(),
  line2: z.string().optional(),
  city: z.string().optional(),
  country: z.string().optional(),
  state: z.string().optional(),
  zipCode: z.string().optional(),
})

const shippingAddressSchema = z.object({
  address: addressSchema.optional(),
  sameAsBilling: z.boolean().default(false),
})

const customTaxSchema = z.object({
  taxCode: z.string().min(1, 'Tax code is required'),
  name: z.string().min(1, 'Tax name is required'),
  rate: z.coerce.number().min(0).max(100),
})

// Base schema with all customer fields (shared between create and edit)
const customerFormFields = z.object({
  name: z.string().default(''),
  // COMPANY (B2B) is the default; INDIVIDUAL (B2C) unlocks the person fields below.
  customerType: z.enum(['COMPANY', 'INDIVIDUAL']).default('COMPANY'),
  firstName: z.string().trim().optional(),
  lastName: z.string().trim().optional(),
  alias: z.string().optional(),
  email: z.string().email().optional().or(z.literal('')),
  invoicingEmail: z.string().email().optional().or(z.literal('')),
  phone: z.string().optional(),
  vatNumber: z.string().optional(),
  legalNumber: z.string().optional(),
  buyerReference: z.string().optional(),
  customTaxes: z.array(customTaxSchema).optional(),
  taxStatus: z.enum(['TAXABLE', 'EXEMPT', 'REVERSE_CHARGE']).default('TAXABLE'),
  exemptionReason: z.string().optional(),
  billingAddress: addressSchema.optional(),
  shippingAddress: shippingAddressSchema.optional(),
  preferredLanguage: z.string().optional(),
  // User-defined custom property values, keyed by definition key. Rendered dynamically.
  customProperties: z.record(z.string(), z.unknown()).optional(),
})

export const individualName = (firstName?: string, lastName?: string) =>
  [firstName?.trim(), lastName?.trim()].filter(Boolean).join(' ')

const validateCustomerIdentity = (
  values: z.infer<typeof customerFormFields>,
  ctx: z.RefinementCtx
) => {
  if (values.customerType === 'INDIVIDUAL') {
    if (!values.firstName) {
      ctx.addIssue({ code: 'custom', path: ['firstName'], message: 'First name is required' })
    }
    if (!values.lastName) {
      ctx.addIssue({ code: 'custom', path: ['lastName'], message: 'Last name is required' })
    }
    if (!values.billingAddress?.country?.trim()) {
      ctx.addIssue({
        code: 'custom',
        path: ['billingAddress', 'country'],
        message: 'Billing country is required',
      })
    }
  } else if (values.name.trim().length < 3) {
    ctx.addIssue({ code: 'custom', path: ['name'], message: 'Name must be at least 3 characters' })
  }
}

export const customerName = (values: z.infer<typeof customerFormFields>) =>
  values.customerType === 'INDIVIDUAL'
    ? individualName(values.firstName, values.lastName)
    : values.name.trim()

export const customerFormSchema = customerFormFields.superRefine(validateCustomerIdentity)

export type CustomerFormSchema = z.infer<typeof customerFormSchema>

// Create schema extends base with required invoicing entity and currency
export const createCustomerSchema = customerFormFields
  .extend({
    invoicingEntity: z.string().min(1, 'Invoicing entity is required'),
    currency: z.string().min(1, 'Currency is required'),
    connectedAccountId: z.string().optional(),
  })
  .superRefine(validateCustomerIdentity)

export type CreateCustomerSchema = z.infer<typeof createCustomerSchema>

// Edit schema is just the base (no invoicing entity/currency changes)
export const editCustomerSchema = customerFormSchema

export type EditCustomerSchema = z.infer<typeof editCustomerSchema>
