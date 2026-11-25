import { Button } from '@md/ui'
import { ArrowUpDownIcon, Clock } from 'lucide-react'
import { type ReactNode } from 'react'

import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { SubscriptionMinimumCommitment } from '@/features/subscriptions/SubscriptionMinimumCommitment'
import { SubscriptionSectionCard } from '@/features/subscriptions/SubscriptionSectionCard'
import {
  billedSlots,
  feeBillingPeriod,
  formatSubscriptionFee,
  slotFeeTotal,
  slotFeeWords,
} from '@/features/subscriptions/utils/fees'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { getSlotsValue } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type {
  SubscriptionComponent,
  SubscriptionDetails,
  SubscriptionFee,
  SubscriptionFee_SlotSubscriptionFee,
} from '@/rpc/api/subscriptions/v1/models_pb'

/** A seat charge to change: a plan charge, or an add-on of the subscription. */
export interface SlotUpdateData {
  priceComponentId?: string
  addOnId?: string
  unit: string
  currentSlots: number
  unitRate: string
  minSlots?: number
  maxSlots?: number
}

/** A seat ledger: the managed quantity counted (a fee recorded before quantities: its unit). */
export interface SeatLedger {
  unit: string
  quantityId?: string
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
    const slot = fee.fee.value
    const seats = billedSlots(slot, currentSlots)
    // A model prices the count (seats included, tiers, packs): read in words, never one rate.
    const priced = slot.model?.model.case
      ? `${seats} ${slot.unit}(s) · ${slotFeeWords(slot, currency)}`
      : `${formatCurrencyNoRounding(Number(slot.unitRate || 0), currency)} × ${seats} ${slot.unit}(s)`
    return (
      <div className="space-y-0.5">
        <div className="text-foreground tabular-nums">
          {formatCurrencyNoRounding(slotFeeTotal(slot, currentSlots).toString(), currency)}
        </div>
        <div className="text-xs text-muted-foreground">{priced}</div>
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

/** A seat charge's count, read by its quantity: charges counting one quantity share it. */
const useSeats = (subscriptionId: string, slot: SubscriptionFee_SlotSubscriptionFee | null) =>
  useQuery(
    getSlotsValue,
    { subscriptionId, unit: slot?.unit ?? '', quantityId: slot?.quantityId || undefined },
    { enabled: !!slot && Boolean(subscriptionId), refetchOnWindowFocus: false }
  )

/** Its ledger, then "change the seats" for those who manage subscriptions. */
const SeatActions = ({
  slot,
  target,
  current,
  loading,
  canManage,
  onUpdateSlot,
  onViewTransactions,
}: {
  slot: SubscriptionFee_SlotSubscriptionFee
  target: { priceComponentId?: string; addOnId?: string }
  current?: number
  loading: boolean
  canManage: boolean
  onUpdateSlot: (data: SlotUpdateData) => void
  onViewTransactions: (ledger: SeatLedger) => void
}) => (
  <div className="flex items-center gap-1">
    <Button
      variant="ghost"
      size="sm"
      onClick={() => onViewTransactions({ unit: slot.unit, quantityId: slot.quantityId })}
      disabled={loading}
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
          onUpdateSlot({
            ...target,
            unit: slot.unit,
            currentSlots: current ?? slot.initialSlots,
            unitRate: slot.unitRate,
            minSlots: slot.minSlots,
            maxSlots: slot.maxSlots,
          })
        }
        disabled={loading}
        className="h-7 px-2 text-brand"
        title="Upgrade or downgrade"
      >
        <ArrowUpDownIcon className="h-4 w-4" />
      </Button>
    )}
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
  onViewTransactions: (ledger: SeatLedger) => void
}) => {
  const slot = component.fee?.fee.case === 'slot' ? component.fee.fee.value : null
  const slotsValueQuery = useSeats(subscriptionId, slot)

  const actions = slot && component.priceComponentId && (
    <SeatActions
      slot={slot}
      target={{ priceComponentId: component.priceComponentId }}
      current={slotsValueQuery.data?.currentValue}
      loading={slotsValueQuery.isLoading}
      canManage={canManage}
      onUpdateSlot={onUpdateSlot}
      onViewTransactions={onViewTransactions}
    />
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

/** An add-on: one charging per seat bills its quantity's count, changed as a plan's seats. */
const AddOnRow = ({
  addOn,
  subscriptionId,
  currency,
  canManage,
  onUpdateSlot,
  onViewTransactions,
}: {
  addOn: SubscriptionDetails['addOns'][number]
  subscriptionId: string
  currency: string
  canManage: boolean
  onUpdateSlot: (data: SlotUpdateData) => void
  onViewTransactions: (ledger: SeatLedger) => void
}) => {
  const slot = addOn.fee?.fee.case === 'slot' ? addOn.fee.fee.value : null
  const slotsValueQuery = useSeats(subscriptionId, slot)
  return (
    <TermsRow
      name={addOn.name}
      kind={`${periodLabel[feeBillingPeriod(addOn.period, addOn.fee)]} · Add-on`}
      value={
        <SubscriptionFeeDetail
          fee={addOn.fee}
          currency={currency}
          currentSlots={slot ? slotsValueQuery.data?.currentValue : undefined}
          isLoadingSlots={!!slot && slotsValueQuery.isLoading}
        />
      }
      action={
        slot && addOn.addOnId ? (
          <SeatActions
            slot={slot}
            target={{ addOnId: addOn.addOnId }}
            current={slotsValueQuery.data?.currentValue}
            loading={slotsValueQuery.isLoading}
            canManage={canManage}
            onUpdateSlot={onUpdateSlot}
            onViewTransactions={onViewTransactions}
          />
        ) : undefined
      }
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
  onViewTransactions: (ledger: SeatLedger) => void
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
          <AddOnRow
            key={addOn.id || i}
            addOn={addOn}
            subscriptionId={subscription.id}
            currency={currency}
            canManage={canManage}
            onUpdateSlot={onUpdateSlot}
            onViewTransactions={onViewTransactions}
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
