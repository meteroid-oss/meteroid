import { z } from 'zod'

const slotStructureSchema = z.object({
  unitName: z.string().min(1, 'Unit name is required'),
})

const capacityStructureSchema = z.object({
  metricId: z.string().min(1, 'Metric ID is required'),
})

const usageStructureSchema = z.object({
  metricId: z.string().min(1, 'Metric ID is required'),
  model: z.enum(['PER_UNIT', 'TIERED', 'VOLUME', 'PACKAGE', 'MATRIX']),
})

const extraRecurringStructureSchema = z.object({
  billingType: z.enum(['ARREAR', 'ADVANCE']),
})

export const createProductSchema = z
  .object({
    name: z.string().min(3),
    description: z.string().optional(),
    productFamilyLocalId: z.string().optional(),
    feeType: z.enum(['RATE', 'SLOT', 'CAPACITY', 'USAGE', 'EXTRA_RECURRING', 'ONE_TIME']).optional(),
    slotStructure: slotStructureSchema.optional(),
    capacityStructure: capacityStructureSchema.optional(),
    usageStructure: usageStructureSchema.optional(),
    extraRecurringStructure: extraRecurringStructureSchema.optional(),
  })
  .superRefine((data, ctx) => {
    if (data.feeType === 'SLOT' && !data.slotStructure) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Slot structure is required for Slot fee type',
        path: ['slotStructure', 'unitName'],
      })
    }
    if (data.feeType === 'CAPACITY' && !data.capacityStructure) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Capacity structure is required for Capacity fee type',
        path: ['capacityStructure', 'metricId'],
      })
    }
    if (data.feeType === 'USAGE' && !data.usageStructure) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Usage structure is required for Usage fee type',
        path: ['usageStructure', 'metricId'],
      })
    }
    if (data.feeType === 'EXTRA_RECURRING' && !data.extraRecurringStructure) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Billing type is required for Extra Recurring fee type',
        path: ['extraRecurringStructure', 'billingType'],
      })
    }
  })

export const createProductFamily = z.object({
  name: z.string().min(3),
  localId: z.string().min(3),
  description: z.string().optional(),
})

export const getByLocalId = z.object({
  localId: z.string(),
})

export const listByPlanLocalId = z.object({
  planLocalId: z.string(),
})
