import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import { toast } from 'sonner'

import { AdvancedSection } from '@/components/form'
import { ExtraActions } from '@/features/plans/MoreForPlan'
import { useIsDraftVersion } from '@/features/plans/hooks/usePlan'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { PlanType, TrialConfig } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanOverview,
  listPlans,
  updatePlanTrial,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

interface TrialProps {
  config?: TrialConfig
  currentPlanId: string
  currentPlanVersionId: string
  planType: PlanType
  onDone: () => void
}

// Select items can't carry an empty value.
const THIS_PLAN = 'current'

interface Draft {
  days: string
  free: boolean
  /** THIS_PLAN, or the plan whose features the trial gives. */
  featuresOf: string
}

/** The version's trial: read on a published version, a small form on a draft. */
export const PlanTrial = ({
  config,
  currentPlanId,
  currentPlanVersionId,
  planType,
  onDone,
}: TrialProps) => {
  const isDraft = useIsDraftVersion()
  const planName = usePlanName()
  const isFree = planType === PlanType.FREE

  if (!isDraft) {
    return (
      <div className="space-y-3 text-sm">
        {config && config.durationDays > 0 ? (
          <TrialSentence
            days={config.durationDays}
            free={config.trialIsFree}
            featuresOf={
              config.trialingPlanId && config.trialingPlanId !== currentPlanId
                ? planName(config.trialingPlanId)
                : undefined
            }
            isFreePlan={isFree}
          />
        ) : (
          <p className="text-muted-foreground">No trial.</p>
        )}
        <p className="text-xs text-muted-foreground">
          The trial is part of the version: change it in a draft.
        </p>
      </div>
    )
  }

  return (
    <TrialForm
      config={config}
      currentPlanId={currentPlanId}
      currentPlanVersionId={currentPlanVersionId}
      planType={planType}
      onDone={onDone}
    />
  )
}

const usePlanName = () => {
  const plans = useQuery(listPlans, {}).data?.plans
  return (id: string) => plans?.find(plan => plan.id === id)?.name ?? 'another plan'
}

const TrialForm = ({
  config,
  currentPlanId,
  currentPlanVersionId,
  planType,
  onDone,
}: TrialProps) => {
  const id = useId()
  const queryClient = useQueryClient()
  const isFreePlan = planType === PlanType.FREE
  const existing = !!config && config.durationDays > 0
  const [draft, setDraft] = useState<Draft>(() => ({
    days: String(config && config.durationDays > 0 ? config.durationDays : 14),
    free: config?.trialIsFree ?? true,
    featuresOf:
      config?.trialingPlanId && config.trialingPlanId !== currentPlanId
        ? config.trialingPlanId
        : THIS_PLAN,
  }))
  const set = (patch: Partial<Draft>) => setDraft(prev => ({ ...prev, ...patch }))

  const plans = useQuery(listPlans, {}).data?.plans ?? []
  // A trial gives a paid plan's features; this plan's own, by default.
  const others = plans.filter(p => p.id !== currentPlanId && p.planType !== PlanType.FREE)
  const planName = (planId: string) => plans.find(p => p.id === planId)?.name ?? 'another plan'

  const days = Number(draft.days)
  const valid = Number.isInteger(days) && days >= 1 && days <= 9999

  const mutation = useMutation(updatePlanTrial, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: getPlanOverview.parent, cardinality: undefined }),
      })
      onDone()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The trial could not be saved')),
  })

  const save = () =>
    mutation.mutate({
      planId: currentPlanId,
      planVersionId: currentPlanVersionId,
      trial: {
        durationDays: days,
        trialingPlanId: draft.featuresOf === THIS_PLAN ? undefined : draft.featuresOf,
        // A free plan's trial never bills.
        trialIsFree: isFreePlan || draft.free,
      },
    })
  const remove = () =>
    mutation.mutate({ planId: currentPlanId, planVersionId: currentPlanVersionId, trial: undefined })

  const featuresSelect = (
    <Select value={draft.featuresOf} onValueChange={featuresOf => set({ featuresOf })}>
      <SelectTrigger id={`${id}-features`} className="h-9">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={THIS_PLAN}>This plan&apos;s</SelectItem>
        {others.map(plan => (
          <SelectItem key={plan.id} value={plan.id}>
            {plan.name}&apos;s
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )

  return (
    <form
      onSubmit={event => {
        event.preventDefault()
        if (valid) save()
      }}
      className="space-y-4 text-sm"
    >
      <div className="grid grid-cols-2 gap-3">
        <div className="space-y-1.5">
          <Label htmlFor={`${id}-days`} className="text-xs font-normal text-muted-foreground">
            Length
          </Label>
          <div className="relative">
            <Input
              id={`${id}-days`}
              type="number"
              inputMode="numeric"
              min={1}
              max={9999}
              step={1}
              autoFocus
              value={draft.days}
              onChange={event => set({ days: event.target.value })}
              className="h-9 pr-12 [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
            />
            <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-xs text-muted-foreground">
              days
            </span>
          </div>
        </div>
        {isFreePlan ? (
          <div className="space-y-1.5">
            <Label htmlFor={`${id}-features`} className="text-xs font-normal text-muted-foreground">
              Features of
            </Label>
            {featuresSelect}
          </div>
        ) : (
          <div className="space-y-1.5">
            <Label htmlFor={`${id}-billing`} className="text-xs font-normal text-muted-foreground">
              During the trial
            </Label>
            <Select
              value={draft.free ? 'free' : 'billed'}
              onValueChange={value => set({ free: value === 'free' })}
            >
              <SelectTrigger id={`${id}-billing`} className="h-9">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="free">Nothing is billed</SelectItem>
                <SelectItem value="billed">Billed as usual</SelectItem>
              </SelectContent>
            </Select>
          </div>
        )}
      </div>

      {!isFreePlan && others.length > 0 && (
        <AdvancedSection
          label="Another plan's features"
          defaultOpen={draft.featuresOf !== THIS_PLAN}
          summary={draft.featuresOf === THIS_PLAN ? 'none' : planName(draft.featuresOf)}
        >
          <div className="space-y-1.5">
            <Label htmlFor={`${id}-features`} className="text-xs font-normal text-muted-foreground">
              During the trial, subscribers get the features of
            </Label>
            {featuresSelect}
          </div>
        </AdvancedSection>
      )}

      <div className="rounded-md bg-muted/50 px-3 py-2.5">
        {valid ? (
          <TrialSentence
            days={days}
            free={isFreePlan || draft.free}
            featuresOf={draft.featuresOf === THIS_PLAN ? undefined : planName(draft.featuresOf)}
            isFreePlan={isFreePlan}
          />
        ) : (
          <p className="text-xs text-destructive">Enter a whole number of days, from 1 to 9999.</p>
        )}
      </div>

      <ExtraActions
        start={
          existing && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="text-destructive hover:text-destructive"
              disabled={mutation.isPending}
              onClick={remove}
            >
              Remove trial
            </Button>
          )
        }
      >
        <Button type="button" variant="secondary" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" size="sm" disabled={!valid || mutation.isPending}>
          {existing ? 'Save' : 'Add trial'}
        </Button>
      </ExtraActions>
    </form>
  )
}

/** The trial in plain words, then what checkout asks for. */
const TrialSentence = ({
  days,
  free,
  featuresOf,
  isFreePlan,
}: {
  days: number
  free: boolean
  /** Another plan's name when the trial gives its features. */
  featuresOf?: string
  isFreePlan: boolean
}) => {
  const length = `${days} ${days === 1 ? 'day' : 'days'}`
  const withFeatures = featuresOf ? (
    <>
      {' '}
      with <strong className="font-medium text-foreground">{featuresOf}</strong>&apos;s features
    </>
  ) : null
  const [sentence, note] = isFreePlan
    ? [
        <>
          New subscribers get <strong className="font-medium text-foreground">{length}</strong>
          {withFeatures}, then continue on this free plan.
        </>,
        'No payment method needed.',
      ]
    : free
      ? [
          <>
            New subscribers get <strong className="font-medium text-foreground">{length} free</strong>
            {withFeatures}, then billing starts.
          </>,
          'Checkout collects a payment method; the first charge is when the trial ends.',
        ]
      : [
          <>
            New subscribers are <strong className="font-medium text-foreground">billed from day one</strong>
            {featuresOf ? (
              <>
                {' '}
                and get <strong className="font-medium text-foreground">{featuresOf}</strong>&apos;s
                features for {length}
              </>
            ) : (
              <> with a {length} introductory period</>
            )}
            .
          </>,
          'Only the features change during the trial, not the price or billing period.',
        ]
  return (
    <div className="space-y-0.5">
      <p className="text-sm text-muted-foreground">{sentence}</p>
      <p className="text-xs text-muted-foreground/80">{note}</p>
    </div>
  )
}
