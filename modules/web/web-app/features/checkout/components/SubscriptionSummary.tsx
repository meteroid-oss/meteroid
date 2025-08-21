import { Checkout } from '@/rpc/portal/checkout/v1/models_pb'
import { formatCurrency } from '@/utils/numbers'

// Helper to format dates
const formatDate = (dateString: string): string => {
  const date = new Date(dateString)
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })
}

const SubscriptionSummary: React.FC<{ checkoutData: Checkout }> = ({ checkoutData }) => {
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
  const hasCoupons = appliedCoupons && appliedCoupons.length > 0

  // Determine if there are manual discounts
  const hasDiscounts = discountAmount && discountAmount > 0

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

        {/* Display manual discounts if any */}
        {hasDiscounts ? (
          <div className="flex justify-between text-green-600 mb-1">
            <div className="text-sm">Discount</div>
            <div>-{formatCurrency(discountAmount, currency)}</div>
          </div>
        ) : null}

        {/* Display applied coupons if any */}
        {hasCoupons &&
          appliedCoupons.map((coupon, index) => (
            <div key={index} className="flex justify-between text-green-600 mb-1">
              <div className="text-sm">
                {coupon.couponName} ({coupon.couponCode})
              </div>
              <div>-{formatCurrency(coupon.amount, currency)}</div>
            </div>
          ))}

        {/* Display tax breakdown if any */}
        {hasTaxes && (
          <>
            <div className="border-t border-gray-100 mt-2 pt-2">
              {taxBreakdown.map((tax, index) => (
                <div key={index} className="flex justify-between text-sm mb-1">
                  <div className="text-muted-foreground">
                    {tax.name} ({parseFloat(tax.rate).toFixed(2)}%)
                  </div>
                  <div className="text-muted-foreground">
                    {formatCurrency(tax.amount, currency)}
                  </div>
                </div>
              ))}
            </div>
            <div className="flex justify-between mt-2 font-medium">
              <div>Total Tax</div>
              <div>{formatCurrency(taxAmount, currency)}</div>
            </div>
          </>
        )}

        {/* Promotion code link */}
        {!hasCoupons && (
          <button className="text-blue-600 text-xs font-medium mt-2">Add promotion code</button>
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
