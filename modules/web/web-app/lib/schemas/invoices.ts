import { z } from 'zod'

export const invoiceLineSchema = z.object({
  product: z.string().min(1, "Product is required"),
  startDate: z.date({ required_error: "Start date is required" }),
  endDate: z.date({ required_error: "End date is required" }),
  quantity: z.number().min(0.01, "Quantity must be greater than 0"),
  unitPrice: z.number().min(0.01, "Unit price must be greater than 0"),
  taxRate: z.number().min(0).max(100, "Tax rate must be between 0 and 100"),
})

export const createInvoiceSchema = z.object({
  customerId: z.string(),
  invoiceDate: z.date(),
  dueDate: z.date().optional(),
  currency: z.string(),
  purchaseOrder: z.string().optional(),
  discount: z.number().min(0, "Discount must be 0 or greater").optional(),
  lines: z.array(invoiceLineSchema).min(1, "At least one invoice line is required"),
})

export type InvoiceLineSchema = z.infer<typeof invoiceLineSchema>
export type CreateInvoiceSchema = z.infer<typeof createInvoiceSchema>
