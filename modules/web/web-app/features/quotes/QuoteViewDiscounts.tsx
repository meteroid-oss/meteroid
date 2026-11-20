import { couponDiscountLabel } from '@/features/productCatalog/coupons/discountLabel'

import type { QuoteCouponTerms } from '@/rpc/api/quotes/v1/models_pb'

/** How long the discount lasts; a promotional credit has no duration. */
export const couponDurationLabel = (terms: QuoteCouponTerms): string | undefined => {
  if (terms.discount?.discountType.case === 'credit') return undefined
  const periods = terms.recurringValue
  if (periods === undefined) return 'forever'
  return periods === 1 ? 'once' : `first ${periods} billing periods`
}

/** "20% off · first 3 billing periods" */
export const couponTermsLabel = (terms: QuoteCouponTerms) =>
  [couponDiscountLabel(terms.discount), couponDurationLabel(terms)].filter(Boolean).join(' · ')

export const QuoteDiscounts = ({ coupons }: { coupons: QuoteCouponTerms[] }) => {
  if (coupons.length === 0) return null
  return (
    <section className="mb-8">
      <h3 className="border-b border-border pb-3 text-md font-medium text-foreground">Discounts</h3>
      <dl className="text-sm">
        {coupons.map((coupon, index) => (
          <div
            key={`${coupon.code}-${index}`}
            className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border-b border-border px-3 py-3 last:border-b-0 sm:px-4"
          >
            <dt className="min-w-0">
              <span className="font-mono font-medium text-foreground">{coupon.code}</span>
              {coupon.description && (
                <div className="mt-0.5 whitespace-pre-line text-xs text-muted-foreground">
                  {coupon.description}
                </div>
              )}
            </dt>
            <dd className="text-right text-foreground">{couponTermsLabel(coupon)}</dd>
          </div>
        ))}
      </dl>
    </section>
  )
}
