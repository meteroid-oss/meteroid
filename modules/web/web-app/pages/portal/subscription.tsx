import { useMutation } from '@connectrpc/connect-query'
import { Skeleton } from '@md/ui'
import { AlertCircle, ArrowLeft, ArrowRight, Check, ChevronDown, ChevronRight, Clock } from 'lucide-react'
import { useState } from 'react'
import { useParams, useSearchParams } from 'react-router-dom'

import { useQuery } from '@/lib/connectrpc'
import { FeeType } from '@/rpc/api/prices/v1/models_pb'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  cancelScheduledEvent,
  confirmPlanChange,
  getSubscriptionComponentUsage,
  getSubscriptionDetails,
  getUpcomingInvoice,
  listAvailablePlans,
  previewPlanChange,
} from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'
import { PendingEventType } from '@/rpc/portal/subscription/v1/subscription_pb'
import { formatCurrency, formatCurrencyNoRounding } from '@/utils/numbers'
import { UsageBarChartDisplay } from '@/features/subscriptions/UsageBarChart'
import { useForceTheme } from 'providers/ThemeProvider'

import type {
  AvailablePlan,
  ComponentFee,
  PortalUpcomingInvoice,
} from '@/rpc/portal/subscription/v1/subscription_pb'

type Mode = 'idle' | 'changing' | 'confirmed'

const formatDate = (dateString: string): string => {
  const date = new Date(dateString)
  return date.toLocaleDateString('en-US', { month: 'long', day: 'numeric', year: 'numeric' })
}

const cadenceLabel = (cadence: SubscriptionFeeBillingPeriod): string => {
  switch (cadence) {
    case SubscriptionFeeBillingPeriod.MONTHLY:
      return '/mo'
    case SubscriptionFeeBillingPeriod.QUARTERLY:
      return '/quarter'
    case SubscriptionFeeBillingPeriod.YEARLY:
      return '/yr'
    case SubscriptionFeeBillingPeriod.SEMIANNUAL:
      return '/6mo'
    case SubscriptionFeeBillingPeriod.ONE_TIME:
      return ''
    default:
      return ''
  }
}

const feeTypeLabel = (feeType: FeeType): string => {
  switch (feeType) {
    case FeeType.RATE:
      return 'Rate'
    case FeeType.SLOT:
      return 'Per seat'
    case FeeType.CAPACITY:
      return 'Capacity'
    case FeeType.USAGE:
      return 'Usage-based'
    case FeeType.EXTRA_RECURRING:
      return 'Recurring'
    case FeeType.ONE_TIME:
      return 'One-time'
    default:
      return 'Fee'
  }
}

const formatComponentFee = (fee: ComponentFee, currency: string): string => {
  if (!fee.amount) {
    return feeTypeLabel(fee.feeType)
  }
  const amount = formatCurrencyNoRounding(fee.amount, currency)
  if (fee.feeType === FeeType.SLOT && fee.unit) {
    return `${amount}/${fee.unit}${cadenceLabel(fee.cadence)}`
  }
  return `${amount}${cadenceLabel(fee.cadence)}`
}

export const PortalSubscription = () => {
  useForceTheme('light')
  const { subscriptionId } = useParams<{ subscriptionId: string }>()
  const [searchParams] = useSearchParams()
  const token = searchParams.get('token')

  const [mode, setMode] = useState<Mode>('idle')
  const [selectedPlan, setSelectedPlan] = useState<AvailablePlan | null>(null)
  const [confirmedDate, setConfirmedDate] = useState<string | null>(null)

  const detailsQuery = useQuery(getSubscriptionDetails, { subscriptionId: subscriptionId! })
  const plansQuery = useQuery(
    listAvailablePlans,
    { subscriptionId: subscriptionId! },
    { enabled: mode === 'changing' }
  )
  const previewQuery = useQuery(
    previewPlanChange,
    { subscriptionId: subscriptionId!, newPlanVersionId: selectedPlan?.planVersionId ?? '' },
    { enabled: mode === 'changing' && !!selectedPlan }
  )
  const upcomingInvoiceQuery = useQuery(
    getUpcomingInvoice,
    { subscriptionId: subscriptionId! },
    { enabled: mode === 'idle' && !!detailsQuery.data?.subscription }
  )

  const confirmMutation = useMutation(confirmPlanChange)
  const cancelMutation = useMutation(cancelScheduledEvent)

  const handleBackToPortal = () => {
    if (token) {
      window.location.href = `/portal/customer?token=${token}`
    } else {
      window.history.back()
    }
  }

  const handleSelectPlan = (plan: AvailablePlan) => {
    if (plan.isCurrent) return
    setSelectedPlan(plan)
  }

  const handleConfirm = async () => {
    if (!selectedPlan || !subscriptionId) return
    const res = await confirmMutation.mutateAsync({
      subscriptionId,
      newPlanVersionId: selectedPlan.planVersionId,
    })
    setConfirmedDate(res.scheduledFor)
    setMode('confirmed')
  }

  const handleCancelScheduledEvent = async () => {
    const eventId = detailsQuery.data?.subscription?.pendingEvent?.id
    if (!subscriptionId || !eventId) return
    await cancelMutation.mutateAsync({ subscriptionId, eventId })
    detailsQuery.refetch()
  }

  const handleStartChange = () => {
    setSelectedPlan(null)
    setMode('changing')
  }

  const handleBack = () => {
    setSelectedPlan(null)
    setConfirmedDate(null)
    setMode('idle')
    detailsQuery.refetch()
  }

  if (detailsQuery.error) {
    return (
      <div className="min-h-screen w-full bg-gray-50 flex items-center justify-center">
        <div className="max-w-sm mx-auto px-6 py-12 text-center">
          <div className="w-12 h-12 rounded-full bg-gray-100 flex items-center justify-center mx-auto mb-4">
            <AlertCircle className="h-5 w-5 text-gray-400" />
          </div>
          <h2 className="text-base font-semibold text-gray-900 mb-1.5">Something went wrong</h2>
          <p className="text-sm text-gray-500">
            There may be a connection issue or your session might be expired.
          </p>
        </div>
      </div>
    )
  }

  if (detailsQuery.isLoading || !detailsQuery.data?.subscription) {
    return (
      <div className="min-h-screen bg-gray-50">
        <div className="bg-white border-b border-gray-200">
          <div className="max-w-7xl mx-auto px-6 lg:px-8 h-14 flex items-center">
            <Skeleton height={14} width={100} />
          </div>
        </div>
        <div className="max-w-2xl mx-auto px-6 lg:px-8 py-12">
          <Skeleton height={200} className="rounded-xl" />
        </div>
      </div>
    )
  }

  const sub = detailsQuery.data.subscription

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Header bar */}
      <div className="bg-white border-b border-gray-200">
        <div className="max-w-7xl mx-auto px-6 lg:px-8 h-14 flex items-center">
          <button
            onClick={mode === 'idle' ? handleBackToPortal : handleBack}
            className="flex items-center gap-1.5 text-sm text-gray-500 hover:text-gray-900 transition-colors"
          >
            <ArrowLeft size={16} />
            <span>{mode === 'idle' ? 'Back to portal' : 'Back'}</span>
          </button>
        </div>
      </div>

      {mode === 'idle' && (
        <IdleView
          sub={sub}
          subscriptionId={subscriptionId!}
          onChangePlan={handleStartChange}
          onCancelChange={handleCancelScheduledEvent}
          isCancelling={cancelMutation.isPending}
          upcomingInvoice={upcomingInvoiceQuery.data?.invoice}
          isLoadingInvoice={upcomingInvoiceQuery.isLoading}
        />
      )}
      {mode === 'changing' && (
        <PlanChangeView
          plans={plansQuery.data?.plans ?? []}
          isLoadingPlans={plansQuery.isLoading}
          selectedPlan={selectedPlan}
          onSelectPlan={handleSelectPlan}
          currentPlanName={sub.planName}
          currentHeadlineFee={sub.headlineFee}
          currency={sub.currency}
          preview={previewQuery.data}
          isLoadingPreview={previewQuery.isLoading}
          previewError={previewQuery.error}
          onConfirm={handleConfirm}
          isConfirming={confirmMutation.isPending}
        />
      )}
      {mode === 'confirmed' && (
        <ConfirmedView scheduledFor={confirmedDate} onDone={handleBackToPortal} />
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Idle View — Current subscription overview
// ---------------------------------------------------------------------------

function IdleView({
  sub,
  subscriptionId,
  onChangePlan,
  onCancelChange,
  isCancelling,
  upcomingInvoice,
  isLoadingInvoice,
}: {
  sub: {
    planName: string
    headlineFee?: ComponentFee
    currency: string
    currentPeriodEnd?: string
    status: string
    canChangePlan: boolean
    pendingEvent?: { eventType: PendingEventType; scheduledDate: string; newPlanName?: string; cancelReason?: string }
  }
  subscriptionId: string
  onChangePlan: () => void
  onCancelChange: () => void
  isCancelling: boolean
  upcomingInvoice?: PortalUpcomingInvoice
  isLoadingInvoice: boolean
}) {
  const statusLower = sub.status.toLowerCase().replace('_', ' ')
  const isActive = statusLower === 'active'

  return (
    <div className="max-w-2xl mx-auto px-6 lg:px-8 py-12">
      <div className="bg-white rounded-xl border border-gray-200 shadow-sm overflow-hidden">
        <div className="p-6 sm:p-8">
          <div className="flex items-start justify-between mb-1">
            <p className="text-[11px] font-semibold text-gray-400 uppercase tracking-wider">
              Current plan
            </p>
            <span
              className={`inline-flex items-center gap-1.5 text-xs font-medium px-2.5 py-1 rounded-full ${
                isActive ? 'bg-green-50 text-green-700' : 'bg-gray-100 text-gray-600'
              }`}
            >
              {isActive && <span className="w-1.5 h-1.5 rounded-full bg-green-500" />}
              <span className="capitalize">{statusLower}</span>
            </span>
          </div>

          <h2 className="text-xl font-semibold text-gray-900 mt-2">{sub.planName}</h2>

          {sub.headlineFee && (
            <div className="flex items-baseline gap-1 mt-3">
              <span className="text-3xl font-semibold text-gray-900 tabular-nums">
                {formatCurrencyNoRounding(sub.headlineFee.amount, sub.currency)}
              </span>
              <span className="text-sm text-gray-400 font-medium">
                {cadenceLabel(sub.headlineFee.cadence)}
              </span>
            </div>
          )}

          {sub.currentPeriodEnd && (
            <p className="text-sm text-gray-500 mt-4">
              Current period ends {formatDate(sub.currentPeriodEnd)}
            </p>
          )}
        </div>

        {/* Scheduled event banner */}
        {sub.pendingEvent && (
          <div
            className={`border-t border-gray-100 px-6 sm:px-8 py-4 ${
              sub.pendingEvent.eventType === PendingEventType.CANCEL
                ? 'bg-amber-50/60'
                : 'bg-blue-50/60'
            }`}
          >
            <div className="flex items-center justify-between gap-4">
              <div className="flex items-center gap-3 min-w-0">
                <div
                  className={`w-8 h-8 rounded-full flex items-center justify-center flex-shrink-0 ${
                    sub.pendingEvent.eventType === PendingEventType.CANCEL
                      ? 'bg-amber-100'
                      : 'bg-blue-100'
                  }`}
                >
                  <Clock
                    size={15}
                    className={
                      sub.pendingEvent.eventType === PendingEventType.CANCEL
                        ? 'text-amber-600'
                        : 'text-blue-600'
                    }
                  />
                </div>
                <div className="min-w-0">
                  <p className="text-sm font-medium text-gray-900 truncate">
                    {sub.pendingEvent.eventType === PendingEventType.PLAN_CHANGE
                      ? `Switching to ${sub.pendingEvent.newPlanName}`
                      : 'Cancellation scheduled'}
                  </p>
                  <p className="text-xs text-gray-500 mt-0.5">
                    Effective {formatDate(sub.pendingEvent.scheduledDate)}
                  </p>
                </div>
              </div>
              <button
                onClick={onCancelChange}
                disabled={isCancelling}
                className="text-sm text-gray-500 hover:text-gray-900 font-medium flex-shrink-0 disabled:opacity-50 transition-colors"
              >
                {isCancelling ? 'Revoking...' : 'Revoke'}
              </button>
            </div>
          </div>
        )}

        {/* Change plan CTA */}
        {sub.canChangePlan && !sub.pendingEvent && (
          <div className="border-t border-gray-100 px-6 sm:px-8 py-4">
            <button
              onClick={onChangePlan}
              className="w-full flex items-center justify-center gap-2 text-sm font-semibold text-gray-900 bg-gray-50 hover:bg-gray-100 border border-gray-200 rounded-lg py-2.5 transition-colors"
            >
              Change plan
              <ArrowRight size={15} />
            </button>
          </div>
        )}
      </div>

      {/* Upcoming invoice */}
      {!isLoadingInvoice && upcomingInvoice && subscriptionId && (
        <PortalUpcomingInvoiceCard invoice={upcomingInvoice} subscriptionId={subscriptionId} />
      )}
      {isLoadingInvoice && (
        <Skeleton height={80} className="rounded-xl mt-6" />
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Portal Upcoming Invoice Card
// ---------------------------------------------------------------------------

function PortalUpcomingInvoiceCard({ invoice, subscriptionId }: { invoice: PortalUpcomingInvoice; subscriptionId: string }) {
  const [expanded, setExpanded] = useState(false)
  const currency = invoice.currency

  return (
    <div className="bg-white rounded-xl border border-gray-200 shadow-sm overflow-hidden mt-6">
      <button
        className="w-full px-6 sm:px-8 py-5 flex items-center justify-between cursor-pointer hover:bg-gray-50/50 transition-colors"
        onClick={() => setExpanded(!expanded)}
      >
        <div className="flex items-center gap-3">
          {expanded ? (
            <ChevronDown className="h-4 w-4 text-gray-400" />
          ) : (
            <ChevronRight className="h-4 w-4 text-gray-400" />
          )}
          <div className="text-left">
            <p className="text-sm font-semibold text-gray-900">Upcoming Invoice</p>
            <p className="text-xs text-gray-500 mt-0.5">
              {formatDate(invoice.periodStart)} &mdash; {formatDate(invoice.periodEnd)}
              {invoice.lineItems.length > 0 && (
                <span className="ml-1.5">
                  &middot; {invoice.lineItems.length} item
                  {invoice.lineItems.length !== 1 ? 's' : ''}
                </span>
              )}
            </p>
          </div>
        </div>
        <span className="text-lg font-semibold text-gray-900 tabular-nums">
          {formatCurrency(Number(invoice.total), currency)}
        </span>
      </button>

      {expanded && (
        <div className="border-t border-gray-100">
          {/* Line items */}
          <div className="divide-y divide-gray-100">
            {invoice.lineItems.map((line, idx) => (
              <PortalLineItemRow
                key={line.id || idx}
                line={line}
                currency={currency}
                subscriptionId={subscriptionId}
              />
            ))}
          </div>

          {/* Coupons */}
          {invoice.couponLineItems.length > 0 && (
            <div className="border-t border-gray-100 px-6 sm:px-8 py-3 space-y-1">
              {invoice.couponLineItems.map((coupon, idx) => (
                <div key={idx} className="flex justify-between text-sm">
                  <span className="text-gray-500">Coupon: {coupon.name}</span>
                  <span className="text-green-600 font-medium tabular-nums">
                    -{formatCurrency(Number(coupon.total), currency)}
                  </span>
                </div>
              ))}
            </div>
          )}

          {/* Totals */}
          <div className="border-t border-gray-100 px-6 sm:px-8 py-4 bg-gray-50/50 space-y-1.5">
            {invoice.discount > 0 && (
              <div className="flex justify-between text-sm text-gray-500">
                <span>Discount</span>
                <span className="tabular-nums">
                  -{formatCurrency(Number(invoice.discount), currency)}
                </span>
              </div>
            )}
            {invoice.taxAmount > 0 && (
              <div className="flex justify-between text-sm text-gray-500">
                <span>Tax</span>
                <span className="tabular-nums">
                  {formatCurrency(Number(invoice.taxAmount), currency)}
                </span>
              </div>
            )}
            <div className="flex justify-between text-sm font-semibold text-gray-900 pt-1 border-t border-gray-200">
              <span>Total</span>
              <span className="tabular-nums">
                {formatCurrency(Number(invoice.total), currency)}
              </span>
            </div>
            {invoice.amountDue !== invoice.total && (
              <div className="flex justify-between text-sm font-semibold text-gray-900">
                <span>Amount due</span>
                <span className="tabular-nums">
                  {formatCurrency(Number(invoice.amountDue), currency)}
                </span>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function PortalLineItemRow({
  line,
  currency,
  subscriptionId,
}: {
  line: PortalUpcomingInvoice['lineItems'][number]
  currency: string
  subscriptionId: string
}) {
  const [showUsage, setShowUsage] = useState(false)
  const hasMetric = Boolean(line.metricId)

  const usageQuery = useQuery(
    getSubscriptionComponentUsage,
    { subscriptionId, metricId: line.metricId ?? '' },
    { enabled: showUsage && hasMetric }
  )

  return (
    <div>
      <div className="px-6 sm:px-8 py-3 flex items-center justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <p className="text-sm text-gray-900 font-medium truncate">{line.name}</p>
            {hasMetric && (
              <button
                className="text-[10px] text-blue-600 hover:underline cursor-pointer"
                onClick={() => setShowUsage(!showUsage)}
              >
                {showUsage ? 'hide usage' : 'usage'}
              </button>
            )}
          </div>
          {line.quantity && line.unitPrice && (
            <p className="text-xs text-gray-500 mt-0.5 tabular-nums">
              {line.quantity} &times; {line.unitPrice}
            </p>
          )}
        </div>
        <span className="text-sm font-medium text-gray-900 tabular-nums ml-4">
          {formatCurrency(Number(line.subtotal), currency)}
        </span>
      </div>
      {showUsage && hasMetric && usageQuery.data && (
        <div className="px-6 sm:px-8 pb-3">
          <UsageBarChartDisplay data={usageQuery.data} />
        </div>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Plan Change View — Centered single-column layout
// ---------------------------------------------------------------------------

function PlanChangeView({
  plans,
  isLoadingPlans,
  selectedPlan,
  onSelectPlan,
  currentPlanName,
  currentHeadlineFee,
  currency,
  preview,
  isLoadingPreview,
  previewError,
  onConfirm,
  isConfirming,
}: {
  plans: AvailablePlan[]
  isLoadingPlans: boolean
  selectedPlan: AvailablePlan | null
  onSelectPlan: (plan: AvailablePlan) => void
  currentPlanName: string
  currentHeadlineFee?: ComponentFee
  currency: string
  preview:
    | {
        preview?: {
          effectiveDate: string
          componentChanges: {
            componentName: string
            isNew: boolean
            currentFee?: ComponentFee
            newFee?: ComponentFee
          }[]
        }
        newPlanName: string
      }
    | undefined
  isLoadingPreview: boolean
  previewError: unknown
  onConfirm: () => void
  isConfirming: boolean
}) {
  const targetPlan = selectedPlan
  const targetHeadlineFee = targetPlan?.headlineFee

  return (
    <div className="max-w-xl mx-auto px-6 py-10">
      <h1 className="text-2xl font-semibold text-gray-900">Change your plan</h1>
      <p className="text-sm text-gray-500 mt-1.5">
        Select the plan that best fits your needs.
      </p>

      {/* Plan cards */}
      {isLoadingPlans ? (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mt-8">
          {[1, 2, 3].map(i => (
            <Skeleton key={i} height={120} className="rounded-xl" />
          ))}
        </div>
      ) : plans.length === 0 ? (
        <div className="text-center py-16">
          <p className="text-sm text-gray-500">
            No other plans are available for self-service switching.
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mt-8">
          {plans.map(plan => (
            <PlanCard
              key={plan.planId}
              plan={plan}
              currency={currency}
              isSelected={selectedPlan?.planId === plan.planId}
              onSelect={() => onSelectPlan(plan)}
            />
          ))}
        </div>
      )}

      {/* Inline summary — shown when a plan is selected */}
      {targetPlan && (
        <div className="mt-8 space-y-4">
          {/* Order details */}
          <div className="bg-white rounded-xl border border-gray-200 p-5">
            <p className="text-sm font-semibold text-gray-900 mb-3">Order details</p>
            <div className="flex items-start gap-3">
              <div className="flex-1 min-w-0">
                <p className="text-[10px] font-semibold text-gray-400 uppercase tracking-wider">
                  Current
                </p>
                <p className="text-sm font-semibold text-gray-900 mt-1 truncate">
                  {currentPlanName}
                </p>
                {currentHeadlineFee && (
                  <p className="text-xs text-gray-500 mt-0.5 tabular-nums">
                    {formatComponentFee(currentHeadlineFee, currency)}
                  </p>
                )}
              </div>
              <div className="flex items-center px-1 pt-5">
                <ArrowRight size={16} className="text-gray-300" />
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-[10px] font-semibold text-blue-600 uppercase tracking-wider">
                  New plan
                </p>
                <p className="text-sm font-semibold text-gray-900 mt-1 truncate">
                  {preview?.newPlanName ?? targetPlan.planName}
                </p>
                {targetHeadlineFee && (
                  <p className="text-xs text-gray-500 mt-0.5 tabular-nums">
                    {formatComponentFee(targetHeadlineFee, currency)}
                  </p>
                )}
              </div>
            </div>
          </div>

          {/* Effective date */}
          {isLoadingPreview && (
            <Skeleton height={56} className="rounded-xl" />
          )}

          {!isLoadingPreview && !!previewError && (
            <div className="bg-white rounded-xl border border-gray-200 p-4 flex items-center gap-3">
              <AlertCircle className="h-4 w-4 text-gray-400 flex-shrink-0" />
              <p className="text-sm text-gray-500">Unable to load preview</p>
            </div>
          )}

          {!isLoadingPreview && !previewError && preview?.preview && (
            <div className="bg-white rounded-xl border border-gray-200 p-4">
              <div className="flex gap-3">
                <Clock size={16} className="text-gray-400 mt-0.5 flex-shrink-0" />
                <p className="text-sm text-gray-700">
                  Your plan will change to{' '}
                  <span className="font-medium">{preview.newPlanName}</span> on your next billing
                  cycle ({formatDate(preview.preview.effectiveDate)}).
                </p>
              </div>
            </div>
          )}

          {/* Confirm button */}
          <button
            onClick={onConfirm}
            disabled={isConfirming || isLoadingPreview || !!previewError}
            className="w-full text-sm font-semibold text-white bg-gray-900 hover:bg-gray-800 rounded-lg py-3 disabled:opacity-50 transition-colors"
          >
            {isConfirming
              ? 'Confirming...'
              : `Confirm ${preview?.newPlanName ?? targetPlan.planName}`}
          </button>

          <p className="text-[11px] text-gray-400 text-center leading-relaxed">
            Your current features remain active until the transition date.
          </p>
        </div>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Plan Card
// ---------------------------------------------------------------------------

function PlanCard({
  plan,
  currency,
  isSelected,
  onSelect,
}: {
  plan: AvailablePlan
  currency: string
  isSelected: boolean
  onSelect: () => void
}) {
  const isCurrent = plan.isCurrent

  return (
    <button
      onClick={onSelect}
      disabled={isCurrent}
      className={`relative text-left rounded-xl p-5 transition-all duration-150 border-2 outline-none ${
        isCurrent
          ? 'bg-gray-50/80 border-gray-200 cursor-default'
          : isSelected
            ? 'bg-white border-gray-900 shadow-sm'
            : 'bg-white border-gray-200 hover:border-gray-300 hover:shadow-sm cursor-pointer'
      }`}
    >
      {/* Top-right indicator */}
      {isCurrent && (
        <span className="absolute top-4 right-4 text-[10px] font-semibold uppercase tracking-wider text-gray-400">
          Current
        </span>
      )}
      {isSelected && (
        <span className="absolute top-4 right-4">
          <span className="w-5 h-5 rounded-full bg-gray-900 flex items-center justify-center">
            <Check size={12} className="text-white" strokeWidth={3} />
          </span>
        </span>
      )}

      <h3 className="text-base font-semibold text-gray-900 pr-12">{plan.planName}</h3>

      {plan.headlineFee && (
        <p className="text-sm text-gray-900 font-medium mt-1 tabular-nums">
          {formatComponentFee(plan.headlineFee, currency)}
        </p>
      )}

      {plan.description && (
        <p className="text-xs text-gray-500 mt-1.5 line-clamp-2 leading-relaxed">
          {plan.description}
        </p>
      )}
    </button>
  )
}

// ---------------------------------------------------------------------------
// Confirmed View
// ---------------------------------------------------------------------------

function ConfirmedView({
  scheduledFor,
  onDone,
}: {
  scheduledFor: string | null
  onDone: () => void
}) {
  return (
    <div className="max-w-md mx-auto px-6 py-24 text-center">
      <div className="inline-flex items-center justify-center w-14 h-14 rounded-full bg-green-50 mb-5">
        <Check size={28} className="text-green-500" />
      </div>
      <h2 className="text-xl font-semibold text-gray-900 mb-2">Plan change confirmed</h2>
      {scheduledFor && (
        <p className="text-sm text-gray-500 mb-8 leading-relaxed">
          Your plan will switch on {formatDate(scheduledFor)}.
          <br />
          Your current features remain active until then.
        </p>
      )}
      <button
        onClick={onDone}
        className="text-sm font-semibold text-white bg-gray-900 hover:bg-gray-800 rounded-lg px-8 py-2.5 transition-colors"
      >
        Back to portal
      </button>
    </div>
  )
}

