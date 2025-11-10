import { z } from 'zod'

export const invoiceLineSchema = z.object({
  product: z.string().min(1, "Product is required"),
  startDate: z.date({ required_error: "Start date is required" }),
  endDate: z.date({ required_error: "End date is required" }),
  quantity: z.number().min(0.01, "Quantity must be greater than 0"),
  unitPrice: z.number().min(0.01, "Unit price must be greater than 0"),
  taxRate: z.number().min(0).max(100, "Tax rate must be between 0 and 100"),
}).refine(
  (data) => data.endDate > data.startDate,
  {
    message: "End date must be after start date",
    path: ["endDate"],
  }
)

export const createInvoiceSchema = z.object({
  customerId: z.string(),
  invoiceDate: z.date(),
  dueDate: z.date().optional(),
  currency: z.string(),
  purchaseOrder: z.string().optional(),
  discount: z.number().min(0, "Discount must be 0 or greater").optional(),
  lines: z.array(invoiceLineSchema).min(1, "At least one invoice line is required"),
})

const baseLineItemObject = z.object({
  lineItemId: z.string().optional(), // if provided, update existing line item
  name: z.string().min(1, "Name is required"),
  startDate: z.date({ required_error: "Start date is required" }),
  endDate: z.date({ required_error: "End date is required" }),
  taxRate: z.number().min(0).max(100, "Tax rate must be between 0 and 100"),
  description: z.string().optional(),
  metricId: z.string().optional(), 
})

const validateLineItemDates = {
  message: "End date must be after start date",
  path: ["endDate"],
}

export const baseLineItemSchema = baseLineItemObject.refine(
  (data) => data.endDate > data.startDate,
  validateLineItemDates
)

export const updateInvoiceLineSchema = baseLineItemObject.extend({
  quantity: z.number().min(0, "Quantity must be 0 or greater"),
  unitPrice: z.number().min(0, "Unit price must be 0 or greater"),
}).refine(
  (data) => data.endDate > data.startDate,
  validateLineItemDates
)

export const updateInvoiceLineWithSublinesSchema = baseLineItemSchema

export type SubLineItem = {
  id: string
  name: string
  total: bigint
  quantity: string
  unitPrice: string
  sublineAttributes?: {
    case: "tiered" | "volume" | "matrix" | "package" | undefined
    value?: unknown
  }
}

// (from proto)
export type OriginalLineItem = {
  id: string
  name: string
  subtotal: bigint
  taxRate: string
  unitPrice?: string
  startDate: string
  endDate: string
  quantity?: string
  subLineItems?: SubLineItem[]
  isProrated: boolean
  priceComponentId?: string
  productId?: string
  metricId?: string
  description?: string
}
 
export type UpdateInvoiceLineSchemaRegular = z.infer<typeof updateInvoiceLineSchema> & {
  _originalItem?: OriginalLineItem
}

export type UpdateInvoiceLineSchemaWithSublines = z.infer<typeof updateInvoiceLineWithSublinesSchema> & {
  _originalItem?: OriginalLineItem
}

// Schema for updating customer details
export const updateInlineCustomerSchema = z.object({
  refreshFromCustomer: z.boolean().default(false),
  name: z.string().optional(),
  billingAddress: z.object({
    line1: z.string().optional(),
    line2: z.string().optional(),
    city: z.string().optional(),
    country: z.string().optional(),
    state: z.string().optional(),
    zipCode: z.string().optional(),
  }).optional(),
  vatNumber: z.string().optional(),
  email: z.string().email("Invalid email").optional().or(z.literal("")),
})

export const updateInvoiceSchema = z.object({
  id: z.string(),
  memo: z.string().optional(),
  reference: z.string().optional(),
  purchaseOrder: z.string().optional(),
  dueDate: z.date().optional(),
  discount: z.number().min(0, "Discount must be 0 or greater").optional(),
  lines: z.array(z.union([updateInvoiceLineSchema, updateInvoiceLineWithSublinesSchema])).min(1, "At least one invoice line is required").optional(),
  customerDetails: updateInlineCustomerSchema.optional(),
})

export type InvoiceLineSchema = z.infer<typeof invoiceLineSchema>
export type CreateInvoiceSchema = z.infer<typeof createInvoiceSchema>
export type UpdateInvoiceLineSchema = z.infer<typeof updateInvoiceLineSchema>
export type UpdateInlineCustomerSchema = z.infer<typeof updateInlineCustomerSchema>
export type UpdateInvoiceSchema = z.infer<typeof updateInvoiceSchema>
