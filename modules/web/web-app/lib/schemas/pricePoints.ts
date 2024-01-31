import { z } from 'zod'

const durationSchema = z.object({
  value: z.number().int().positive(),
  unit: z.enum(['DAY', 'MONTH']),
})

const trialPriceRampSchema = z.object({
  name: z.string().nonempty('Name is required').max(256),
  duration: durationSchema,
  netTerms: z.number().nonnegative().max(180),
})

const pricedProduct = z.object({
  productId: z.string(),
  discount: z.any().optional(),
  minimum: z.any().optional(),
  customPricingUnitId: z.string().optional(),
  recurringFees: z.array(z.any()),
  usageBasedFees: z.array(z.any()),
})
export type PricedProductSchema = z.infer<typeof pricedProduct>

/*

        name: string;
  durationInBillingPeriods?: number | undefined;
  discount: ProductDiscount | undefined;
  minimum: AmountMinimum | undefined;
  freeCreditAmount?: number | undefined;
  idx: number;

  */

const priceRampSchema = z.object({
  name: z.string().nonempty('Name is required').max(256),
  durationInBillingPeriods: z.number().positive().optional(),
  discount: z.any().optional(),
  minimum: z.any().optional(),
  freeCreditAmount: z.number().optional(),
  idx: z.number(),
})
export type PriceRampSchema = z.infer<typeof priceRampSchema>

export const createPricePointSchema = z.object({
  pricePointName: z.string().nonempty('Name is required').max(256),
  planExternalId: z.string(),
  currency: z.string(),
  cycle: z.enum(['FOREVER', 'FIXED']),
  frequency: z.enum(['ANNUAL', 'MONTHLY', 'QUARTERLY', 'SEMI_ANNUAL', 'SEMI_MONTHLY']),
  periodStart: z.enum(['DAY_OF_MONTH', 'START_OF_PLAN']),
  trial: trialPriceRampSchema.optional(),
  phases: z.array(priceRampSchema), // TODO refine, last duration is empty of cycle is forever
  products: z.array(pricedProduct),
})

export const listPricePointsSchema = z.object({
  planExternalId: z.string(),
})

/*


TODO GAsp 


> Product Family 

>> Product Item
>> Plan
>> Features


Sequence applies prices to Product Items, allowing to reuse a price across plans (and therefore update all at once)
Probably we can simply update from the Product Item (list all PricedProducts of type Plan / Custom, select all => change price => apply)


Priced Product Item


// Plan page => 2 display mode : vertical table, or listing


Create plan

(Pick product items)


Priced component : 
- Flat fee (charge) : one time, recurring
- Usage-based fees : only in arrear, near real-time, no limit => ex: MAU, bandwith, storage, ...
- Variable : Entitlement/Addon-based => fit seats etc in there. Customers can choose a number of unit ahead (seats, licenses etc), changing the price
(variable allows to have different prices for a plan based on a pre-selected number, ex: seats, clusters, licenses)
=> that's just commitment ?
=> also, it means that subscriptions must provide a value for that variable
- Calculated usage => "Entitlement-based fee" ? : like usage-based, but lower throughput, consistent, allows limits, no in-house aggregation ? 
Do we want that or keeping Limits out of the Billing (so maybe duplicate the user metric into seats & used seats )

How does that translate with BYO metering ? => same, the query layer is the same ?



Entitlements/Features




*/
