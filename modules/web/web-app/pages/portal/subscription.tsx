import { Skeleton } from '@md/ui'
import { useMutation } from '@connectrpc/connect-query'
import { AlertCircle, ArrowLeft, ArrowRight, Check, Clock, Info } from 'lucide-react'
import { useState } from 'react'
import { useParams, useSearchParams } from 'react-router-dom'

import { useQuery } from '@/lib/connectrpc'
import {
  cancelScheduledPlanChange,
  confirmPlanChange,
  getSubscriptionDetails,
  listAvailablePlans,
  previewPlanChange,
} from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'
import type { AvailablePlan } from '@/rpc/portal/subscription/v1/subscription_pb'
import { formatCurrency } from '@/utils/numbers'
import { useForceTheme } from 'providers/ThemeProvider'

type Mode = 'idle' | 'changing' | 'confirmed'

const formatDate = (dateString: string): string => {
  const date = new Date(dateString)
  return date.toLocaleDateString('en-US', { month: 'long', day: 'numeric', year: 'numeric' })
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

  const confirmMutation = useMutation(confirmPlanChange)
  const cancelMutation = useMutation(cancelScheduledPlanChange)

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

  const handleCancelScheduledChange = async () => {
    if (!subscriptionId) return
    await cancelMutation.mutateAsync({ subscriptionId })
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
          onChangePlan={handleStartChange}
          onCancelChange={handleCancelScheduledChange}
          isCancelling={cancelMutation.isPending}
        />
      )}
      {mode === 'changing' && (
        <PlanChangeView
          plans={plansQuery.data?.plans ?? []}
          isLoadingPlans={plansQuery.isLoading}
          selectedPlan={selectedPlan}
          onSelectPlan={handleSelectPlan}
          currentPlanName={sub.planName}
          currentMrr={sub.mrrCents}
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
  onChangePlan,
  onCancelChange,
  isCancelling,
}: {
  sub: {
    planName: string
    mrrCents: bigint
    currency: string
    currentPeriodEnd?: string
    status: string
    canChangePlan: boolean
    scheduledPlanChange?: string
    scheduledPlanChangeDate?: string
  }
  onChangePlan: () => void
  onCancelChange: () => void
  isCancelling: boolean
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

          <div className="flex items-baseline gap-1 mt-3">
            <span className="text-3xl font-semibold text-gray-900 tabular-nums">
              {formatCurrency(Number(sub.mrrCents), sub.currency)}
            </span>
            <span className="text-sm text-gray-400 font-medium">/ mo</span>
          </div>

          {sub.currentPeriodEnd && (
            <p className="text-sm text-gray-500 mt-4">
              Current period ends {formatDate(sub.currentPeriodEnd)}
            </p>
          )}
        </div>

        {/* Scheduled change banner */}
        {sub.scheduledPlanChange && (
          <div className="border-t border-gray-100 bg-blue-50/60 px-6 sm:px-8 py-4">
            <div className="flex items-center justify-between gap-4">
              <div className="flex items-center gap-3 min-w-0">
                <div className="w-8 h-8 rounded-full bg-blue-100 flex items-center justify-center flex-shrink-0">
                  <Clock size={15} className="text-blue-600" />
                </div>
                <div className="min-w-0">
                  <p className="text-sm font-medium text-gray-900 truncate">
                    Switching to {sub.scheduledPlanChange}
                  </p>
                  {sub.scheduledPlanChangeDate && (
                    <p className="text-xs text-gray-500 mt-0.5">
                      Effective {formatDate(sub.scheduledPlanChangeDate)}
                    </p>
                  )}
                </div>
              </div>
              <button
                onClick={onCancelChange}
                disabled={isCancelling}
                className="text-sm text-gray-500 hover:text-gray-900 font-medium flex-shrink-0 disabled:opacity-50 transition-colors"
              >
                {isCancelling ? 'Cancelling...' : 'Cancel'}
              </button>
            </div>
          </div>
        )}

        {/* Change plan CTA */}
        {sub.canChangePlan && !sub.scheduledPlanChange && (
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
    </div>
  )
}

// ---------------------------------------------------------------------------
// Plan Change View — Two-panel: plan selection + live preview summary
// ---------------------------------------------------------------------------

function PlanChangeView({
  plans,
  isLoadingPlans,
  selectedPlan,
  onSelectPlan,
  currentPlanName,
  currentMrr,
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
  currentMrr: bigint
  currency: string
  preview:
    | {
        preview?: {
          effectiveDate: string
          componentChanges: {
            componentName: string
            action: string
            oldValue?: string
            newValue?: string
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
  return (
    <div className="flex flex-col lg:flex-row min-h-[calc(100vh-57px)]">
      {/* Left — Plan selection */}
      <div className="flex-1 px-6 lg:px-12 py-10 lg:py-12 overflow-y-auto">
        <div className="max-w-[640px] mx-auto lg:mx-0 lg:ml-auto lg:mr-12">
          <h1 className="text-2xl font-semibold text-gray-900">Change your plan</h1>
          <p className="text-sm text-gray-500 mt-1.5">
            Select the plan that best fits your needs.
          </p>

          {isLoadingPlans ? (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 mt-8">
              {[1, 2, 3].map(i => (
                <Skeleton key={i} height={140} className="rounded-xl" />
              ))}
            </div>
          ) : plans.length === 0 ? (
            <div className="text-center py-16">
              <p className="text-sm text-gray-500">
                No other plans are available for self-service switching.
              </p>
            </div>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 mt-8">
              {plans.map(plan => (
                <PlanCard
                  key={plan.planId}
                  plan={plan}
                  isSelected={selectedPlan?.planId === plan.planId}
                  onSelect={() => onSelectPlan(plan)}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Right — Summary panel */}
      {selectedPlan && (
        <div className="w-full lg:w-[420px] lg:min-w-[420px] bg-white border-t lg:border-t-0 lg:border-l border-gray-200 lg:shadow-[-1px_0_3px_rgba(0,0,0,0.04)]">
          <SummaryPanel
            currentPlanName={currentPlanName}
            currentMrr={currentMrr}
            currency={currency}
            newPlanName={preview?.newPlanName ?? selectedPlan.planName}
            preview={preview?.preview}
            isLoading={isLoadingPreview}
            error={previewError}
            onConfirm={onConfirm}
            isConfirming={isConfirming}
          />
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
  isSelected,
  onSelect,
}: {
  plan: AvailablePlan
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
      {plan.description && (
        <p className="text-sm text-gray-500 mt-1.5 line-clamp-3 leading-relaxed">
          {plan.description}
        </p>
      )}

      {!isCurrent && !isSelected && (
        <p className="text-sm font-medium text-gray-900 mt-4 flex items-center gap-1">
          Select
          <ArrowRight size={14} />
        </p>
      )}
    </button>
  )
}

// ---------------------------------------------------------------------------
// Summary Panel — Plan comparison, changes preview, confirmation
// ---------------------------------------------------------------------------

function SummaryPanel({
  currentPlanName,
  currentMrr,
  currency,
  newPlanName,
  preview,
  isLoading,
  error,
  onConfirm,
  isConfirming,
}: {
  currentPlanName: string
  currentMrr: bigint
  currency: string
  newPlanName: string
  preview?: {
    effectiveDate: string
    componentChanges: {
      componentName: string
      action: string
      oldValue?: string
      newValue?: string
    }[]
  }
  isLoading: boolean
  error: unknown
  onConfirm: () => void
  isConfirming: boolean
}) {
  return (
    <div className="p-6 lg:p-8 h-full flex flex-col lg:overflow-y-auto">
      <h2 className="text-lg font-semibold text-gray-900">Summary of Changes</h2>

      {/* Plan comparison */}
      <div className="mt-6">
        <SectionLabel>Plan comparison</SectionLabel>
        <div className="bg-gray-50 rounded-lg p-4 mt-2.5 flex gap-3">
          <div className="flex-1 min-w-0">
            <p className="text-[10px] font-semibold text-gray-400 uppercase tracking-wider">
              Current
            </p>
            <p className="text-sm font-semibold text-gray-900 mt-1 truncate">{currentPlanName}</p>
            <p className="text-xs text-gray-500 mt-0.5 tabular-nums">
              {formatCurrency(Number(currentMrr), currency)}/mo
            </p>
          </div>
          <div className="flex items-center px-1">
            <ArrowRight size={16} className="text-gray-300" />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-[10px] font-semibold text-blue-600 uppercase tracking-wider">
              New plan
            </p>
            <p className="text-sm font-semibold text-gray-900 mt-1 truncate">{newPlanName}</p>
          </div>
        </div>
      </div>

      {/* Loading state */}
      {isLoading && (
        <div className="mt-6 space-y-4 flex-1">
          <Skeleton height={64} className="rounded-lg" />
          <Skeleton height={16} width={120} />
          <Skeleton height={48} className="rounded-lg" />
          <Skeleton height={48} className="rounded-lg" />
        </div>
      )}

      {/* Error state */}
      {!isLoading && error && (
        <div className="mt-6 flex-1 flex flex-col items-center justify-center py-8">
          <AlertCircle className="h-5 w-5 text-gray-400 mb-2" />
          <p className="text-sm text-gray-500">Unable to load preview</p>
        </div>
      )}

      {/* Preview loaded */}
      {!isLoading && !error && preview && (
        <>
          {/* Effective date info */}
          <div className="mt-6 bg-blue-50 border border-blue-100 rounded-lg p-4">
            <div className="flex gap-3">
              <Info size={16} className="text-blue-500 mt-0.5 flex-shrink-0" />
              <div>
                <p className="text-sm font-medium text-blue-900">
                  Changes take effect on {formatDate(preview.effectiveDate)}
                </p>
                <p className="text-xs text-blue-700/80 mt-0.5 leading-relaxed">
                  Your new plan will start at the end of your current billing period. You will not
                  lose any prepaid days from your current plan.
                </p>
              </div>
            </div>
          </div>

          {/* Component changes */}
          {preview.componentChanges.length > 0 && (
            <div className="mt-6">
              <SectionLabel>Changes</SectionLabel>
              <div className="mt-2.5 space-y-0 divide-y divide-gray-100">
                {preview.componentChanges.map((change, i) => (
                  <div key={i} className="flex items-center justify-between py-3 first:pt-0">
                    <div className="flex items-center gap-2.5 min-w-0">
                      <ComponentDot action={change.action} />
                      <span className="text-sm text-gray-700 truncate">
                        {change.componentName}
                      </span>
                    </div>
                    {(change.oldValue || change.newValue) && (
                      <div className="text-sm flex items-center gap-1.5 flex-shrink-0 ml-3">
                        {change.oldValue && (
                          <span className="text-gray-400 tabular-nums">{change.oldValue}</span>
                        )}
                        {change.oldValue && change.newValue && (
                          <ArrowRight size={12} className="text-gray-300" />
                        )}
                        {change.newValue && (
                          <span className="font-medium text-gray-900 tabular-nums">
                            {change.newValue}
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Confirm CTA */}
          <div className="mt-auto pt-8">
            <div className="border-t border-gray-100 pt-6">
              <button
                onClick={onConfirm}
                disabled={isConfirming}
                className="w-full text-sm font-semibold text-white bg-gray-900 hover:bg-gray-800 rounded-lg py-3 disabled:opacity-50 transition-colors"
              >
                {isConfirming ? 'Confirming...' : 'Confirm Change'}
              </button>
              <p className="text-[11px] text-gray-400 text-center mt-3 leading-relaxed">
                Your current features remain active until the transition date.
              </p>
            </div>
          </div>
        </>
      )}
    </div>
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

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-[11px] font-semibold text-gray-400 uppercase tracking-wider">{children}</p>
  )
}

function ComponentDot({ action }: { action: string }) {
  const color =
    action.toLowerCase() === 'added'
      ? 'bg-green-500'
      : action.toLowerCase() === 'removed'
        ? 'bg-red-400'
        : 'bg-gray-300'
  return <span className={`w-2 h-2 rounded-full ${color} flex-shrink-0`} />
}
