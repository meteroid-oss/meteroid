import { Button, Input } from '@md/ui'
import { useMemo, useState } from 'react'

import { formatSubscriptionFee } from '@/features/subscriptions/utils/fees'
import { formatCurrency, formatCurrencyNoRounding, rateToPercent } from '@/lib/utils/numbers'
import { FeeStructure_BillingType } from '@/rpc/api/prices/v1/models_pb'
import { SubscriptionComponent, SubscriptionFee } from '@/rpc/api/subscriptions/v1/models_pb'
import { Checkout } from '@/rpc/portal/checkout/v1/models_pb'

import type {
  AddOnPurchaseCheckoutContext,
  PlanChangeCheckoutContext,
} from '@/rpc/portal/checkout/v1/checkout_pb'

// Helper to format dates
const formatDate = (dateString: string): string => {
  const date = new Date(dateString)
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })
}

// Label for the right column of orphan components (no charge today but billed later).
// Usage and Recurring(Arrears) are the only fee types that end up here at checkout.
const orphanRightLabel = (fee: SubscriptionFee | undefined): string => {
  if (!fee || !fee.fee.case) return '—'
  if (fee.fee.case === 'usage') return 'Based on usage'
  if (fee.fee.case === 'recurring' && fee.fee.value.billingType === FeeStructure_BillingType.ARREAR) {
    return 'Billed in arrears'
  }
  return '—'
}

interface SubscriptionSummaryProps {
  checkoutData: Checkout
  couponCode: string
  onCouponCodeChange: (code: string) => void
  onApplyCoupon: () => void
  onClearCoupon: () => void
  couponError?: string
  isApplyingCoupon?: boolean
  isPlanChange?: boolean
  isAddonPurchase?: boolean
  planChangeContext?: PlanChangeCheckoutContext
  addonPurchaseContext?: AddOnPurchaseCheckoutContext
}

const SubscriptionSummary: React.FC<SubscriptionSummaryProps> = ({
  checkoutData,
  couponCode,
  onCouponCodeChange,
  onApplyCoupon,
  onClearCoupon,
  couponError,
  isApplyingCoupon,
  isPlanChange,
  isAddonPurchase,
  planChangeContext,
  addonPurchaseContext,
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
    appliedCredits,
    amountDue,
    taxBreakdown,
    appliedCoupons,
  } = checkoutData

  // Get currency from subscription
  const currency = subscription?.subscription?.currency || '?'

  // Index price components by their plan price_component_id so we can render
  // pricing terms (rate, included allowance, tiers, matrix) alongside each line.
  const componentByPriceComponentId = useMemo(() => {
    const map = new Map<string, SubscriptionComponent>()
    for (const c of subscription?.priceComponents ?? []) {
      if (c.priceComponentId) map.set(c.priceComponentId, c)
    }
    return map
  }, [subscription?.priceComponents])

  // Components that produced an invoice line for this period — so we can surface
  // the remainder (e.g. pure usage with 0 consumption at checkout) as rate-only rows.
  const invoicedPriceComponentIds = new Set(
    invoiceLines.map(l => l.priceComponentId).filter((id): id is string => !!id)
  )
  const orphanComponents = (subscription?.priceComponents ?? []).filter(
    c => c.priceComponentId && !invoicedPriceComponentIds.has(c.priceComponentId)
  )

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
          {isPlanChange
            ? `Upgrade to ${planChangeContext?.newPlanName ?? subscription?.subscription?.planName ?? 'Plan'}`
            : isAddonPurchase
              ? `Add ${addonPurchaseContext?.addOnName ?? 'add-on'}`
              : `Subscribe to ${subscription?.subscription?.planName || 'Plan'}`}
        </h1>

        <div className="flex items-baseline">
          <span className="text-2xl font-bold">{formatCurrency(amountDue, currency)}</span>
        </div>

        {isPlanChange ? (
          <>
            <div className="text-sm text-gray-600 mt-1">Prorated amount due</div>
            {planChangeContext && (
              <div className="mt-3 text-xs text-muted-foreground space-y-0.5">
                <div>
                  {planChangeContext.currentPlanName} → {planChangeContext.newPlanName}
                </div>
                {planChangeContext.effectiveDate && (
                  <div>Effective {formatDate(planChangeContext.effectiveDate)}</div>
                )}
              </div>
            )}
          </>
        ) : isAddonPurchase ? (
          <>
            <div className="text-sm text-gray-600 mt-1">Prorated amount due</div>
            {addonPurchaseContext && (
              <div className="mt-3 text-xs text-muted-foreground space-y-0.5">
                <div>{addonPurchaseContext.addOnName}</div>
              </div>
            )}
          </>
        ) : (
          <>
            {/* Show billing frequency TODO */}
            <div className="text-sm text-gray-600 mt-1">Billed monthly</div>

            {isInTrial && (
              <div className="mt-2 inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-blue-100 text-blue-800">
                {subscription?.subscription?.trialDuration} day trial
              </div>
            )}
          </>
        )}
      </div>

      {/* Line Items */}
      <div className=" py-4 space-y-4 text-sm">
        {invoiceLines.map((line, index) => {
          const component = line.priceComponentId
            ? componentByPriceComponentId.get(line.priceComponentId)
            : undefined
          const pricing = component ? formatSubscriptionFee(component.fee, currency) : undefined
          const hasQuantityLine =
            !!line.quantity && !!line.unitPrice && Number(line.quantity) > 1
          const showPricingSublabel = !!pricing && !hasQuantityLine && !pricing.redundantDetails
          return (
            <div key={line.id || index} className="flex justify-between">
              <div>
                <div className="font-medium ">{line.name}</div>
                {line.description && (
                  <div className="text-xs text-muted-foreground">{line.description}</div>
                )}
                {hasQuantityLine && (
                  <div className="text-xs text-muted-foreground">
                    {line.quantity} × {formatCurrencyNoRounding(Number(line.unitPrice), currency)}
                  </div>
                )}
                {showPricingSublabel && pricing && (
                  <div className="text-xs text-muted-foreground">
                    {pricing.amount}
                    {pricing.details ? ` · ${pricing.details}` : ''}
                  </div>
                )}
                {pricing?.breakdown && (
                  <pre className="mt-0.5 text-[11px] text-muted-foreground font-mono whitespace-pre leading-tight">
                    {pricing.breakdown}
                  </pre>
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
          )
        })}

        {/* Components that don't produce a charge today but will on the next invoice
            (usage-based, or recurring billed in arrears). Surface their rate terms
            so the customer understands what they're committing to. */}
        {orphanComponents.map((c, index) => {
          const pricing = formatSubscriptionFee(c.fee, currency)
          const rightLabel = orphanRightLabel(c.fee)
          return (
            <div key={c.id || `orphan-${index}`} className="flex justify-between">
              <div>
                <div className="font-medium">{c.name}</div>
                <div className="text-xs text-muted-foreground">
                  {pricing.amount}
                  {pricing.details && !pricing.redundantDetails ? ` · ${pricing.details}` : ''}
                </div>
                {pricing.breakdown && (
                  <pre className="mt-0.5 text-[11px] text-muted-foreground font-mono whitespace-pre leading-tight">
                    {pricing.breakdown}
                  </pre>
                )}
              </div>
              <div className="text-muted-foreground">{rightLabel}</div>
            </div>
          )
        })}
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

        {/* Promotion code input — hidden for plan change and addon purchase */}
        {!hasCoupons && !isPlanChange && !isAddonPurchase && (
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

      {/* Total & Credits */}
      <div className="border-t border-gray-200 py-4">
        {appliedCredits > BigInt(0) && (
          <>
            <div className="flex justify-between mb-2">
              <div>Total</div>
              <div>{formatCurrency(totalAmount, currency)}</div>
            </div>
            <div className="flex justify-between text-green-600 mb-2">
              <div>Credits applied</div>
              <div>-{formatCurrency(appliedCredits, currency)}</div>
            </div>
          </>
        )}
        <div className="flex justify-between font-medium text-lg">
          <div>{isPlanChange || isAddonPurchase ? 'Amount due' : 'Total due today'}</div>
          <div>{formatCurrency(amountDue, currency)}</div>
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
