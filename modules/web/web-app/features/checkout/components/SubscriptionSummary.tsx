import { Button, Input } from '@md/ui'
import { useState } from 'react'

import { formatCurrency, rateToPercent } from '@/lib/utils/numbers'
import { Checkout } from '@/rpc/portal/checkout/v1/models_pb'

// Helper to format dates
const formatDate = (dateString: string): string => {
  const date = new Date(dateString)
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })
}

interface SubscriptionSummaryProps {
  checkoutData: Checkout
  couponCode: string
  onCouponCodeChange: (code: string) => void
  onApplyCoupon: () => void
  onClearCoupon: () => void
  couponError?: string
  isApplyingCoupon?: boolean
}

const SubscriptionSummary: React.FC<SubscriptionSummaryProps> = ({
  checkoutData,
  couponCode,
  onCouponCodeChange,
  onApplyCoupon,
  onClearCoupon,
  couponError,
  isApplyingCoupon,
}) => {
  const [showCouponInput, setShowCouponInput] = useState(false)
  const {
    subscription,
    invoiceLines,
    tradeName,
    logoUrl,
    subtotalAmount,
    taxAmount,
    discountAmount,
    totalAmount,
    taxBreakdown,
    appliedCoupons,
  } = checkoutData

  // Get currency from subscription
  const currency = subscription?.subscription?.currency || '?'

  // Determine if there are any applied coupons
  const hasCoupons: boolean = appliedCoupons.length > 0

  // Determine if there are manual discounts
  const hasDiscounts = discountAmount > 0

  // Determine if there are taxes
  const hasTaxes = taxBreakdown && taxBreakdown.length > 0

  // Check if subscription is in trial
  const isInTrial =
    subscription?.subscription?.trialDuration && subscription.subscription.trialDuration > 0

  return (
    <div className="text-sm">
      {/* Header with logo and tradeName */}
      <div className=" items-center mb-6 lg:flex hidden">
        <button className="mr-4 text-muted-foreground">←</button>
        {logoUrl && <img src={logoUrl} alt={`${tradeName} logo`} className="h-7 w-auto mr-1" />}
        <span className="font-semibold text-md">{tradeName}</span>
      </div>

      {/* Subscription Title */}
      <div className="mb-6">
        <h1 className="text-base font-normal mb-1 text-muted-foreground">
          Subscribe to {subscription?.subscription?.planName || 'Plan'}
        </h1>

        <div className="flex items-baseline">
          <span className="text-2xl font-bold">{formatCurrency(totalAmount, currency)}</span>
        </div>

        {/* Show billing frequency TODO */}
        <div className="text-sm text-gray-600 mt-1">
          Billed monthly
          {/* {subscription?.subscription?.billingDayAnchor &&
            `, on the ${subscription.subscription.billingDayAnchor}${getOrdinalSuffix(subscription.subscription.billingDayAnchor)} of each month`}

         + TODO renews at X/month
         */}
        </div>

        {isInTrial && (
          <div className="mt-2 inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-blue-100 text-blue-800">
            {subscription?.subscription?.trialDuration} day trial
          </div>
        )}
      </div>

      {/* Line Items */}
      <div className=" py-4 space-y-4 text-sm">
        {invoiceLines.map((line, index) => (
          <div key={line.id || index} className="flex justify-between">
            <div>
              <div className="font-medium ">{line.name}</div>
              {line.description && (
                <div className="text-xs text-muted-foreground">{line.description}</div>
              )}
              {line.quantity && line.unitPrice && Number(line.quantity) > 1 && (
                <div className="text-xs text-muted-foreground">
                  {/* TODO */}
                  {line.quantity} × {formatCurrency(Number(line.unitPrice) * 100, currency)}
                </div>
              )}
              {line.isProrated && line.startDate && line.endDate && (
                <div className="text-xs text-muted-foreground">
                  <span className="">Prorated</span> ({formatDate(line.startDate)} →{' '}
                  {formatDate(line.endDate)})
                </div>
              )}
            </div>
            <div className="font-medium">{formatCurrency(line.subtotal, currency)}</div>
          </div>
        ))}
      </div>

      {/* Subtotal */}
      <div className="border-t border-gray-200 py-4">
        <div className="flex justify-between mb-2">
          <div>Subtotal</div>
          <div>{formatCurrency(subtotalAmount, currency)}</div>
        </div>

        {hasCoupons ? (
          <>
            {appliedCoupons.map((coupon, index) => (
              <div key={index} className="flex justify-between text-green-600 mb-1">
                <div className="text-sm">{coupon.couponCode}</div>
                <div>-{formatCurrency(coupon.amount, currency)}</div>
              </div>
            ))}
          </>
        ) : null}

        {hasDiscounts && !hasCoupons && (
          <>
            <div className="flex justify-between text-green-600 mb-1">
              <div className="text-sm">Discount</div>
              <div>-{formatCurrency(discountAmount, currency)}</div>
            </div>
          </>
        )}

        {/* Display tax breakdown if any */}
        {hasTaxes && (
          <>
            <div className="border-t border-gray-100 mt-2 pt-2">
              {taxBreakdown.map((tax, index) => (
                <div key={index} className="flex justify-between text-sm mb-1">
                  <div className="text-muted-foreground">
                    {tax.name} ({rateToPercent(tax.rate)}%)
                  </div>
                  <div className="text-muted-foreground">
                    {formatCurrency(tax.amount, currency)}
                  </div>
                </div>
              ))}
            </div>
          </>
        )}

        {/* Promotion code input */}
        {!hasCoupons && (
          <div className="mt-2">
            {!showCouponInput ? (
              <Button
                variant="link"
                size="sm"
                onClick={() => setShowCouponInput(true)}
                className="text-blue-600 text-xs font-medium p-0 h-auto"
              >
                Add promotion code
              </Button>
            ) : (
              <div className="flex flex-col gap-1">
                <div className="flex items-center gap-2">
                  <Input
                    value={couponCode}
                    onChange={e => onCouponCodeChange(e.target.value)}
                    placeholder="Enter code"
                    className="flex-1 h-8 text-xs"
                    onKeyDown={e => {
                      if (e.key === 'Enter' && couponCode.trim()) {
                        onApplyCoupon()
                      }
                    }}
                  />
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={onApplyCoupon}
                    disabled={!couponCode.trim() || isApplyingCoupon}
                    className="text-xs"
                  >
                    {isApplyingCoupon ? 'Applying...' : 'Apply'}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setShowCouponInput(false)
                      onCouponCodeChange('')
                    }}
                    className="text-muted-foreground text-xs"
                  >
                    Cancel
                  </Button>
                </div>
                {couponError && <p className="text-destructive text-xs">{couponError}</p>}
              </div>
            )}
          </div>
        )}

        {/* Show applied coupon with remove option */}
        {hasCoupons && (
          <div className="mt-2">
            <Button
              variant="link"
              size="sm"
              onClick={onClearCoupon}
              className="text-muted-foreground text-xs p-0 h-auto"
            >
              Remove coupon
            </Button>
          </div>
        )}
      </div>

      {/* Total */}
      <div className="border-t border-gray-200 py-4">
        <div className="flex justify-between font-medium text-lg">
          <div>Total due today</div>
          <div>{formatCurrency(totalAmount, currency)}</div>
        </div>
        {hasTaxes && (
          <div className="text-xs text-muted-foreground mt-1">
            Includes {formatCurrency(taxAmount, currency)} in taxes
          </div>
        )}
      </div>

      {/* Payment Schedule */}
      {/* {subscription?.subscription?.status === SubscriptionStatus.ACTIVE && (
        <div className="mt-4 text-sm text-muted-foreground">
          The next charge is scheduled for{' '}
          {subscription.subscription.billingStartDate
            ? formatDate(subscription.subscription.billingStartDate)
            : 'your next billing date'}
          , followed by recurring payments.
        </div>
      )} */}
    </div>
  )
}

export { SubscriptionSummary }
