import { AppRouterOutput } from '@/lib/schemas'

export type Plan = AppRouterOutput['plans']['list'][number]
export type PriceComponent = AppRouterOutput['plans']['listPriceComponents'][number]

export type PriceComponentType = NonNullable<NonNullable<PriceComponent['fee']>['fee']>
