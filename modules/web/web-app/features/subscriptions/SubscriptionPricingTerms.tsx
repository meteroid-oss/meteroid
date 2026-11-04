import { Button } from '@md/ui'
import { ArrowUpDownIcon, Clock } from 'lucide-react'
import { type ReactNode } from 'react'

import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { SubscriptionMinimumCommitment } from '@/features/subscriptions/SubscriptionMinimumCommitment'
import { SubscriptionSectionCard } from '@/features/subscriptions/SubscriptionSectionCard'
import { feeBillingPeriod, formatSubscriptionFee } from '@/features/subscriptions/utils/fees'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { getSlotsValue } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type {
  SubscriptionComponent,
  SubscriptionDetails,
  SubscriptionFee,
} from '@/rpc/api/subscriptions/v1/models_pb'

export interface SlotUpdateData {
  priceComponentId: string
  unit: string
  currentSlots: number
  unitRate: string
  minSlots?: number
  maxSlots?: number
}

const periodLabel: Record<SubscriptionFeeBillingPeriod, string> = {
  [SubscriptionFeeBillingPeriod.ONE_TIME]: 'Once',
  [SubscriptionFeeBillingPeriod.MONTHLY]: 'Monthly',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: 'Quarterly',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: 'Semiannual',
  [SubscriptionFeeBillingPeriod.YEARLY]: 'Yearly',
}

const feeTypeLabel = (fee: SubscriptionFee | undefined) => {
  switch (fee?.fee.case) {
    case 'rate':
      return 'Fixed fee'
    case 'oneTime':
      return 'One-time fee'
    case 'recurring':
      return 'Recurring fee'
    case 'capacity':
      return 'Capacity'
    case 'slot':
      return 'Seats'
    case 'usage':
      return 'Usage'
    case 'credit':
      return fee.fee.value.once ? 'Credit' : 'Credit pack'
    default:
      return ''
  }
}

export const SubscriptionFeeDetail = ({
  fee,
  currency,
  currentSlots,
  isLoadingSlots,
}: {
  fee: SubscriptionFee | undefined
  currency: string
  currentSlots?: number
  isLoadingSlots?: boolean
}) => {
  const creditUnits = useCreditUnits()
  if (!fee || !fee.fee.case) {
    return <span className="text-sm text-muted-foreground">-</span>
  }
  if (fee.fee.case === 'slot' && currentSlots !== undefined && !isLoadingSlots) {
    const unitRate = Number(fee.fee.value.unitRate)
    return (
      <div className="space-y-0.5">
        <div className="text-foreground tabular-nums">
          {formatCurrencyNoRounding(currentSlots * unitRate, currency)}
        </div>
        <div className="text-xs text-muted-foreground">
          {formatCurrencyNoRounding(unitRate, currency)} × {currentSlots} {fee.fee.value.unit}(s)
        </div>
      </div>
    )
  }
  const formatted = formatSubscriptionFee(fee, currency, creditUnits)
  return (
    <div className="space-y-0.5">
      <div className="text-foreground">{formatted.amount}</div>
      {formatted.details && (
        <div className="text-xs text-muted-foreground">{formatted.details}</div>
      )}
      {formatted.breakdown && (
        <div className="whitespace-pre-line text-xs text-muted-foreground">
          {formatted.breakdown}
        </div>
      )}
    </div>
  )
}

const TermsRow = ({
  name,
  kind,
  value,
  action,
}: {
  name: ReactNode
  kind?: ReactNode
  value: ReactNode
  action?: ReactNode
}) => (
  <div className="grid grid-cols-1 gap-1 py-2.5 text-sm md:grid-cols-[11rem_9rem_minmax(0,1fr)_auto] md:items-start md:gap-4">
    <span className="font-medium text-foreground">{name}</span>
    <span className="text-muted-foreground">{kind}</span>
    <div className="min-w-0">{value}</div>
    <div className="flex justify-end">{action}</div>
  </div>
)

const PriceComponentRow = ({
  component,
  subscriptionId,
  currency,
  canManage,
  onUpdateSlot,
  onViewTransactions,
}: {
  component: SubscriptionComponent
  subscriptionId: string
  currency: string
  canManage: boolean
  onUpdateSlot: (data: SlotUpdateData) => void
  onViewTransactions: (unit: string) => void
}) => {
  const slot = component.fee?.fee.case === 'slot' ? component.fee.fee.value : null
  const slotsValueQuery = useQuery(
    getSlotsValue,
    { subscriptionId, unit: slot?.unit ?? '' },
    { enabled: !!slot && Boolean(subscriptionId), refetchOnWindowFocus: false }
  )

  const actions = slot && component.priceComponentId && (
    <div className="flex items-center gap-1">
      <Button
        variant="ghost"
        size="sm"
        onClick={() => onViewTransactions(slot.unit)}
        disabled={slotsValueQuery.isLoading}
        className="h-7 w-7 p-0"
        title="View slot transactions"
      >
        <Clock className="h-4 w-4" />
      </Button>
      {canManage && (
        <Button
          variant="ghost"
          size="sm"
          onClick={() =>
            component.priceComponentId &&
            onUpdateSlot({
              priceComponentId: component.priceComponentId,
              unit: slot.unit,
              currentSlots: slotsValueQuery.data?.currentValue ?? slot.initialSlots,
              unitRate: slot.unitRate,
              minSlots: slot.minSlots,
              maxSlots: slot.maxSlots,
            })
          }
          disabled={slotsValueQuery.isLoading}
          className="h-7 px-2 text-brand"
          title="Upgrade or downgrade"
        >
          <ArrowUpDownIcon className="h-4 w-4" />
        </Button>
      )}
    </div>
  )

  return (
    <TermsRow
      name={component.name}
      kind={`${periodLabel[feeBillingPeriod(component.period, component.fee)]} · ${feeTypeLabel(component.fee)}`}
      value={
        <SubscriptionFeeDetail
          fee={component.fee}
          currency={currency}
          currentSlots={slot ? slotsValueQuery.data?.currentValue : undefined}
          isLoadingSlots={!!slot && slotsValueQuery.isLoading}
        />
      }
      action={actions || undefined}
    />
  )
}

/** Components, add-ons, coupons and, with a minimum, the contract terms: one open card. */
export const SubscriptionPricingTerms = ({
  details,
  isFreePlan,
  canManage,
  contractTerm,
  paymentTerms,
  onAmend,
  onUpdateSlot,
  onViewTransactions,
}: {
  details: SubscriptionDetails
  isFreePlan: boolean
  canManage: boolean
  contractTerm?: string
  paymentTerms: ReactNode
  onAmend?: () => void
  onUpdateSlot: (data: SlotUpdateData) => void
  onViewTransactions: (unit: string) => void
}) => {
  const subscription = details.subscription
  if (!subscription) return null
  const currency = subscription.currency
  const empty =
    details.priceComponents.length === 0 &&
    details.addOns.length === 0 &&
    details.appliedCoupons.length === 0 &&
    !details.minimumCommitment

  return (
    <SubscriptionSectionCard
      title="Pricing & terms"
      aside={
        onAmend && (
          <Button size="sm" variant="outline" onClick={onAmend}>
            Amend
          </Button>
        )
      }
    >
      <div className="divide-y divide-border px-4 py-1.5">
        {empty && (
          <p className="py-2.5 text-sm text-muted-foreground">
            {isFreePlan ? 'Free plan · no charges' : 'No charges'}
          </p>
        )}
        {details.priceComponents.map(component => (
          <PriceComponentRow
            key={component.id}
            component={component}
            subscriptionId={subscription.id}
            currency={currency}
            canManage={canManage}
            onUpdateSlot={onUpdateSlot}
            onViewTransactions={onViewTransactions}
          />
        ))}
        {details.addOns.map((addOn, i) => (
          <TermsRow
            key={addOn.id || i}
            name={addOn.name}
            kind={`${periodLabel[feeBillingPeriod(addOn.period, addOn.fee)]} · Add-on`}
            value={<SubscriptionFeeDetail fee={addOn.fee} currency={currency} />}
          />
        ))}
        {details.appliedCoupons.map((coupon, i) => (
          <TermsRow
            key={coupon.coupon?.id || i}
            name={coupon.coupon?.code ?? 'Coupon'}
            kind="Coupon"
            value={
              <span className="text-foreground">
                {coupon.appliedCoupon?.appliedAmount
                  ? `${formatCurrencyNoRounding(coupon.appliedCoupon.appliedAmount, currency)} applied so far`
                  : coupon.coupon?.description || '—'}
              </span>
            }
          />
        ))}
      </div>
      {details.minimumCommitment && (
        <div className="border-t border-border px-4 pb-1.5 pt-3">
          <div className="text-xs font-medium text-muted-foreground">Contract terms</div>
          <div className="divide-y divide-border">
            <div className="py-2.5">
              <SubscriptionMinimumCommitment
                planVersionId={subscription.planVersionId}
                currency={currency}
                minimum={details.minimumCommitment}
              />
            </div>
            {contractTerm && <TermsRow name="Contract term" value={contractTerm} />}
            <TermsRow name="Payment terms" value={paymentTerms} />
          </div>
        </div>
      )}
    </SubscriptionSectionCard>
  )
}
