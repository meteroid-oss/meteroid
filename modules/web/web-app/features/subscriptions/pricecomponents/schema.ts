import { UseFormReturn } from 'react-hook-form'
import { z } from 'zod'

// Shared enums matching protobuf
const BillingPeriodEnum = z.enum(['ONE_TIME', 'MONTHLY', 'QUARTERLY', 'YEARLY'])
const BillingTypeEnum = z.enum(['ARREAR', 'ADVANCE'])
const UpgradePolicyEnum = z.enum(['PRORATED'])
const DowngradePolicyEnum = z.enum(['REMOVE_AT_END_OF_PERIOD'])

// Common schemas
const PriceString = z.string().regex(/^\d+(\.\d{1,4})?$/)
const PositiveInt = z.number().int().positive()

// Fee type schemas
const RateSubscriptionFee = z.object({
  type: z.literal('rate'),
  rate: PriceString,
})

const OneTimeSubscriptionFee = z.object({
  type: z.literal('one_time'),
  rate: PriceString,
  quantity: PositiveInt,
  total: PriceString,
})

const ExtraRecurringSubscriptionFee = z.object({
  type: z.literal('recurring'),
  rate: PriceString,
  quantity: PositiveInt,
  total: PriceString,
  billing_type: BillingTypeEnum,
})

const CapacitySubscriptionFee = z.object({
  type: z.literal('capacity'),
  rate: PriceString,
  included: z.number().int().nonnegative(),
  overage_rate: PriceString,
  metric_id: z.string(),
})

const SlotSubscriptionFee = z.object({
  type: z.literal('slot'),
  unit: z.string(),
  unit_rate: PriceString,
  min_slots: z.number().int().optional(),
  max_slots: z.number().int().optional(),
  initial_slots: z.number().int(),
  upgrade_policy: UpgradePolicyEnum,
  downgrade_policy: DowngradePolicyEnum,
})

const UsageMatrix = z.object({
  rows: z.array(
    z.object({
      per_unit_price: PriceString,
      dimension1: z.object({
        key: z.string(),
        value: z.string(),
      }),
      dimension2: z
        .object({
          key: z.string(),
          value: z.string(),
        })
        .optional(),
    })
  ),
})

const UsageTierRow = z.object({
  first_unit: z.number().int().nonnegative(),
  unit_price: PriceString,
  flat_fee: PriceString.optional(),
  flat_cap: PriceString.optional(),
})

const UsageTieredAndVolume = z.object({
  rows: z.array(UsageTierRow),
  block_size: z.number().int().positive().optional(),
})

const UsagePackage = z.object({
  package_price: PriceString,
  block_size: z.number().int().positive(),
})

const UsageSubscriptionFee = z.object({
  type: z.literal('usage'),
  metric_id: z.string(),
  model: z.discriminatedUnion('type', [
    z.object({ type: z.literal('per_unit'), price: PriceString }),
    z.object({ type: z.literal('tiered'), config: UsageTieredAndVolume }),
    z.object({ type: z.literal('volume'), config: UsageTieredAndVolume }),
    z.object({ type: z.literal('package'), config: UsagePackage }),
    z.object({ type: z.literal('matrix'), config: UsageMatrix }),
  ]),
})

// Combined fee schema with discriminated union
const SubscriptionFeeSchema = z.discriminatedUnion('type', [
  RateSubscriptionFee,
  OneTimeSubscriptionFee,
  ExtraRecurringSubscriptionFee,
  CapacitySubscriptionFee,
  SlotSubscriptionFee,
  UsageSubscriptionFee,
])

// Component schema
const SubscriptionComponentSchema = z.object({
  price_component_id: z.string().optional(),
  product_id: z.string().optional(),
  name: z.string(),
  period: BillingPeriodEnum,
  fee: SubscriptionFeeSchema,
})

// Main subscription schema
const CreateSubscriptionSchema = z.object({
  plan_version_id: z.string(),
  customer_id: z.string(),
  currency: z.string(),
  trial_start_date: z.string().optional(),
  billing_start_date: z.string(),
  billing_end_date: z.string().optional(),
  billing_day: z.number().int().min(1).max(31),
  net_terms: z.number().int().nonnegative(),
  invoice_memo: z.string().optional(),
  invoice_threshold: z.string().optional(),
  components: z.object({
    parameterized_components: z.array(
      z.object({
        component_id: z.string(),
        initial_slot_count: z.number().int().optional(),
        billing_period: BillingPeriodEnum.optional(),
        committed_capacity: z.number().int().optional(),
      })
    ),
    overridden_components: z.array(
      z.object({
        component_id: z.string(),
        component: SubscriptionComponentSchema,
      })
    ),
    extra_components: z.array(SubscriptionComponentSchema),
    remove_components: z.array(z.string()),
  }),
})

// Type inference
type CreateSubscriptionFormData = z.infer<typeof CreateSubscriptionSchema>

// Form context type
type SubscriptionFormContext = UseFormReturn<CreateSubscriptionFormData>

// Helper to transform form data to protobuf format
const transformFormToProto = (data: CreateSubscriptionFormData) => {
  // Transform the form data to match your protobuf structure
  // This is where you'd handle any necessary data transformations
  return {
    ...data,
    components: {
      ...data.components,
      // Transform any nested structures that need special handling
      parameterized_components: data.components.parameterized_components.map(comp => ({
        ...comp,
        // Handle any specific transformations needed for the API
        committed_capacity: comp.committed_capacity?.toString(),
      })),
    },
  }
}

export {
  CreateSubscriptionSchema,
  transformFormToProto,
  type CreateSubscriptionFormData,
  type SubscriptionFormContext,
}
