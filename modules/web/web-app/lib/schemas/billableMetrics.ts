import { O, S, flow } from '@mobily/ts-belt'
import { z } from 'zod'

const unitConversion = z.object({
  factor: z.coerce.number().positive(),
  rounding: z.enum(['NONE', 'UP', 'DOWN', 'NEAREST']),
})
type UnitConversionSchema = typeof unitConversion
type UnitConversionData = z.infer<typeof unitConversion>
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

type SimpleAggregation = {
  aggregationType: 'SUM' | 'MIN' | 'MAX' | 'MEAN' | 'LATEST' | 'COUNT' | 'COUNT_DISTINCT'
  aggregationKey?: string | undefined
  unitConversion?: UnitConversionData
}
type SimpleAggregationSchema = z.ZodObject<
  {
    aggregationType: z.ZodEnum<['SUM', 'MIN', 'MAX', 'MEAN', 'LATEST', 'COUNT', 'COUNT_DISTINCT']>
    aggregationKey: z.ZodOptional<z.ZodString>
    unitConversion: z.ZodOptional<UnitConversionSchema>
  },
  'strip',
  z.ZodTypeAny,
  SimpleAggregation,
  SimpleAggregation
>
const simpleAggregationSchema: z.ZodEffects<SimpleAggregationSchema> = z
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

type DimensionValuesSchema = z.ZodArray<z.ZodString, 'atleastone'>
const dimensionValues: DimensionValuesSchema = z.array(z.string().nonempty()).nonempty()

// We specify some type explicitely to reduce complexity on ts compiler
type Dimension = {
  values: [string, ...string[]]
  key: string
}
type DimensionSchema = z.ZodObject<
  {
    key: z.ZodString
    values: DimensionValuesSchema
  },
  'strip',
  z.ZodTypeAny,
  Dimension,
  Dimension
>
const dimensionSchema: DimensionSchema = z.object({
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

export const createBillableMetricSchema = z.object({
  metricName: z.string().min(3),
  eventCode: z.string().min(3),
  metricDescription: z.string().optional(),
  aggregation: simpleAggregationSchema,
  segmentationMatrix: simpleSegmentationMatrixSchema,
  productFamilyId: z.string(),
  usageGroupKey: z
    .string()
    .optional()
    .nullable()
    .transform(flow(O.map(S.trim), O.filter(s => !S.isEmpty(s)))),
})
export type CreateBillableMetricSchema = typeof createBillableMetricSchema
export type CreateBillableMetricFormData = z.infer<CreateBillableMetricSchema>
