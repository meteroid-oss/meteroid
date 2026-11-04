import { O, S, flow } from '@mobily/ts-belt'
import { z } from 'zod'

const unitConversion = z.object({
  factor: z.coerce.number().positive(),
  rounding: z.enum(['NONE', 'UP', 'DOWN', 'NEAREST']),
})
// caused typecript to lag
// const _aggregationSchema = z.discriminatedUnion('aggregationType', [
//   z.object({ aggregationType: z.literal('COUNT') }),
//   z.object({ aggregationType: z.literal('COUNT_DISTINCT'), distinctOnKey: z.string() }),
//   z.object({
//     aggregationType: z.enum(['SUM', 'MIN', 'MAX', 'MEAN', 'LATEST']),
//     aggregationKey: z.string(),
//     unitConversion: unitConversion.optional(),
//   }),
// ])

const simpleAggregationSchema = z
  .object({
    aggregationType: z.enum(['SUM', 'MIN', 'MAX', 'MEAN', 'LATEST', 'COUNT', 'COUNT_DISTINCT']),
    aggregationKey: z.string().optional(),
    unitConversion: unitConversion.optional(),
  })
  .superRefine((val, ctx) => {
    if (val.aggregationType !== 'COUNT' && !val.aggregationKey) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['aggregationKey'],
        message: 'Aggregation key is required',
      })
    }
  })

const dimensionValues = z.array(z.string().nonempty()).nonempty()

// We specify some type explicitely to reduce complexity on ts compiler
export type Dimension = {
  values: string[]
  key: string
}
const dimensionSchema = z.object({
  key: z.string().nonempty('Required'),
  values: dimensionValues,
})
// caused typecript to lag
// const _segmentationMatrixSchema = z.discriminatedUnion('matrixType', [
//   z.object({ matrixType: z.literal('NONE') }),
//   z.object({ matrixType: z.literal('SINGLE'), dimension: z.string(), values: dimensionValues }),
//   z.object({
//     matrixType: z.literal('DOUBLE'),
//     dimension: dimensionSchema,
//     dimension2: dimensionSchema,
//   }),
//   z.object({
//     matrixType: z.literal('LINKED'),
//     dimensionKey: z.string(),
//     linkedDimensionKey: z.string(),
//     values: z.record(z.string(), dimensionValues),
//   }),
// ])
const simpleSegmentationMatrixSchema = z.object({
  matrixType: z.enum(['NONE', 'SINGLE', 'DOUBLE', 'LINKED']),
  linked: z
    .object({
      dimensionKey: z.string(),
      linkedDimensionKey: z.string(),
      values: z.record(z.string(), dimensionValues),
    })
    .optional(),
  single: dimensionSchema.optional(),
  double: z.object({ dimension1: dimensionSchema, dimension2: dimensionSchema }).optional(),
})

export type SimpleSegmentationMatrixFormData = z.infer<typeof simpleSegmentationMatrixSchema>

// Pre-aggregation property filter. EQUAL/NOT_EQUAL are the single-value forms of IN/NOT_IN;
// the UI keeps all four for clarity and they collapse to IN/NOT_IN at the engine boundary.
export const metricFilterSchema = z.object({
  property: z.string().nonempty('Required'),
  op: z.enum(['EQUAL', 'NOT_EQUAL', 'IN', 'NOT_IN']),
  values: dimensionValues,
})
export type MetricFilterFormData = z.infer<typeof metricFilterSchema>

// Mirrors server validation: trimmed, max 32 chars, no control chars; singular and plural go together.
const unitLabelText = z
  .string()
  .trim()
  .max(32, 'Must be 32 characters or fewer')
  .regex(/^[^\p{Cc}]*$/u, 'Must not contain control characters')
  .optional()

export const unitLabelFields = {
  unitSingular: unitLabelText,
  unitPlural: unitLabelText,
}

export const refineUnitLabel = (
  val: { unitSingular?: string; unitPlural?: string },
  ctx: z.RefinementCtx
) => {
  const hasSingular = !!val.unitSingular
  const hasPlural = !!val.unitPlural
  if (hasSingular && !hasPlural) {
    ctx.addIssue({
      code: 'custom',
      path: ['unitPlural'],
      message: 'Plural is required with a singular',
    })
  } else if (hasPlural && !hasSingular) {
    ctx.addIssue({
      code: 'custom',
      path: ['unitSingular'],
      message: 'Singular is required with a plural',
    })
  }
}

export const createBillableMetricSchema = z
  .object({
    metricName: z.string().min(3),
    eventCode: z.string().min(3),
    metricDescription: z.string().optional(),
    aggregation: simpleAggregationSchema,
    segmentationMatrix: simpleSegmentationMatrixSchema,
    filters: z.array(metricFilterSchema).optional(),
    productFamilyId: z.string(),
    usageGroupKey: z
      .string()
      .optional()
      .nullable()
      .transform(
        flow(
          O.map(S.trim),
          O.filter(s => !S.isEmpty(s))
        )
      ),
    ...unitLabelFields,
  })
  .superRefine(refineUnitLabel)
export type CreateBillableMetricSchema = typeof createBillableMetricSchema
export type CreateBillableMetricFormData = z.infer<CreateBillableMetricSchema>

export const updateBillableMetricSchema = z.object({
  id: z.string(),
  metricName: z.string().min(3).optional(),
  metricDescription: z.string().optional().nullable(),
  unitConversion: unitConversion.optional().nullable(),
  segmentationMatrix: simpleSegmentationMatrixSchema.optional(),
  filters: z.array(metricFilterSchema).optional(),
  usageGroupKey: z
    .string()
    .optional()
    .nullable()
    .transform(
      flow(
        O.map(S.trim),
        O.filter(s => !S.isEmpty(s))
      )
    ),
})
export type UpdateBillableMetricSchema = typeof updateBillableMetricSchema
export type UpdateBillableMetricFormData = z.infer<UpdateBillableMetricSchema>
