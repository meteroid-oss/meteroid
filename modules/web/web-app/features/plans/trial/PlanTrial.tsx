import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { forwardRef, useState } from 'react'
import { Controller } from 'react-hook-form'
import { z } from 'zod'

import { useIsDraftVersion } from '@/features/plans/hooks/usePlan'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { PlanType, TrialConfig } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanOverview,
  listPlans,
  updatePlanTrial,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'

interface TrialProps {
  config?: TrialConfig
  currentPlanId: string
  currentPlanVersionId: string
  planType: PlanType
}

export const PlanTrial = ({
  config,
  currentPlanId,
  currentPlanVersionId,
  planType,
}: TrialProps) => {
  const isDraft = useIsDraftVersion()
  const [isEditing, setIsEditing] = useState(false)

  if (isEditing) {
    return (
      <PlanTrialForm
        config={config}
        currentPlanId={currentPlanId}
        currentPlanVersionId={currentPlanVersionId}
        planType={planType}
        cancel={() => setIsEditing(false)}
        afterSubmit={() => setIsEditing(false)}
      />
    )
  }

  return (
    <div className="flex flex-row gap-2 items-center text-sm ">
      <PlanTrialReadonly config={config} currentPlanId={currentPlanId} planType={planType} />
      {isDraft && (
        <Button type="button" variant="link" onClick={() => setIsEditing(true)}>
          Edit
        </Button>
      )}
    </div>
  )
}

const formSchema = z.object({
  durationDays: z.number().int().optional(),
  trialingPlanId: z.union([z.string(), z.literal('current')]).optional(),
  trialIsFree: z.boolean().optional(),
})

interface TrialConfigSentenceProps {
  config?: TrialConfig
  currentPlanId: string
  currentPlanVersionId: string
  planType: PlanType
  afterSubmit: () => void
  cancel: () => void
}
export function PlanTrialForm({
  config,
  currentPlanId,
  currentPlanVersionId,
  planType,
  afterSubmit,
  cancel,
}: TrialConfigSentenceProps) {
  const methods = useZodForm({
    schema: formSchema,
    defaultValues: {
      durationDays: config?.durationDays ?? 0,
      trialingPlanId: config?.trialingPlanId,
      trialIsFree: config?.trialIsFree ?? true,
    },
  })

  const queryClient = useQueryClient()

  const mutation = useMutation(updatePlanTrial, {
    onSuccess: () => {
      afterSubmit()
      queryClient.invalidateQueries({
        queryKey: [getPlanOverview.service.typeName],
      })
    },
  })

  const plans = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const data = methods.watch()

  const plansOptions = (plans.data?.plans ?? []).map(plan => {
    const isCurrent = plan.id === currentPlanId

    return {
      value: isCurrent ? 'current' : plan.id,
      name: isCurrent ? 'current' : plan.name,
      type: plan.planType,
      status: plan.planStatus,
    }
  })

  const nonFreePlansOptions = plansOptions.filter(p => p.type !== PlanType.FREE)
  const isFreePlan = planType === PlanType.FREE

  return (
    <div className="space-y-4">
      <div>
        <Form {...methods}>
          <form
            onSubmit={methods.handleSubmit(data => {
              const trial =
                data?.durationDays && data.durationDays > 0
                  ? {
                      trialingPlanId:
                        data.trialingPlanId === 'current' ? undefined : data.trialingPlanId,
                      durationDays: data.durationDays,
                      // Free plans are always free, paid plans use the toggle
                      trialIsFree: isFreePlan ? true : (data.trialIsFree ?? true),
                    }
                  : undefined

              return mutation.mutateAsync({
                planId: currentPlanId,
                planVersionId: currentPlanVersionId,
                trial,
              })
            })}
            className="space-y-4 text-sm"
          >
            {!data.durationDays || data.durationDays <= 0 ? (
              <p className="text-sm">
                No trial configured.
                <Button
                  type="button"
                  variant="link"
                  onClick={() =>
                    methods.reset({
                      durationDays: 14,
                      trialIsFree: true,
                    })
                  }
                >
                  Add trial
                </Button>
              </p>
            ) : (
              <div className="text-sm space-y-2">
                <p>
                  Users get{' '}
                  <Controller
                    name="durationDays"
                    control={methods.control}
                    render={({ field }) => <EditableSpan {...field} type="number" />}
                  />{' '}
                  days
                  {/* Trialing plan selector - for giving premium features during trial */}
                  <Controller
                    name="trialingPlanId"
                    control={methods.control}
                    render={({ field }) => (
                      <>
                        {' '}
                        with{' '}
                        <EditableSpan
                          {...field}
                          options={nonFreePlansOptions}
                          emptyMessage="current"
                          quotes
                        />{' '}
                        features
                      </>
                    )}
                  />
                  {/* Free/Paid toggle - only for paid plans */}
                  {!isFreePlan && (
                    <Controller
                      name="trialIsFree"
                      control={methods.control}
                      render={({ field }) => (
                        <>
                          {' '}
                          <EditableSpan
                            onChange={val => field.onChange(val === 'free')}
                            value={field.value ? 'free' : 'paid'}
                            options={[
                              { value: 'free', name: 'for free' },
                              { value: 'paid', name: 'while being charged' },
                            ]}
                            emptyMessage="for free"
                          />
                        </>
                      )}
                    />
                  )}
                  , then continue on this plan.
                  <Button
                    type="button"
                    variant="link"
                    className="text-destructive"
                    onClick={() =>
                      methods.reset({
                        durationDays: 0,
                      })
                    }
                  >
                    Remove trial
                  </Button>
                </p>
                {/* Contextual help based on plan type */}
                <p className="text-muted-foreground text-xs">
                  {isFreePlan ? (
                    <>No payment method will be required.</>
                  ) : data.trialIsFree ? (
                    <>
                      If checkout and automatic charge are set, payment method will be collected at checkout and a charge
                      will be made when the trial ends.
                    </>
                  ) : (
                    <>This is a paid trial. Only the resolved features are impacted, not the billing period nor the price.</>
                  )}
                </p>
              </div>
            )}

            {methods.formState.errors && (
              <div className="text-[0.8rem] font-medium text-destructive">
                {methods.formState.errors.durationDays && (
                  <p>
                    Invalid duration : {String(methods.formState.errors.durationDays?.message)}
                  </p>
                )}
                {methods.formState.errors.trialingPlanId && (
                  <p>
                    Invalid trial plan : {String(methods.formState.errors.trialingPlanId?.message)}
                  </p>
                )}
                {methods.formState.errors.root && (
                  <p>Error : {String(methods.formState.errors.root?.message)}</p>
                )}
              </div>
            )}

            <div>
              <Button
                type="button"
                variant="link"
                className="text-foreground px-0 mr-4"
                onClick={cancel}
              >
                Cancel
              </Button>
              <Button type="submit" variant="link" className="px-0">
                Save
              </Button>
            </div>
          </form>
        </Form>
      </div>
    </div>
  )
}

interface PlanTrialReadonlyProps {
  config?: TrialConfig
  currentPlanId: string
  planType?: PlanType
}
export function PlanTrialReadonly({ config, currentPlanId, planType }: PlanTrialReadonlyProps) {
  const { data: plansData, isLoading } = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const resolvePlanName = (id: string): string => {
    if (isLoading) return '...'
    const plan = plansData?.plans.find(p => p.id === id)
    return plan?.name ?? 'unknown'
  }

  const isCurrentPlan = (planId?: string) => !planId || planId === currentPlanId
  const isFreePlan = planType === PlanType.FREE

  if (!config) {
    return <p className="text-sm">No trial configured.</p>
  }

  const renderTrialDescription = () => {
    const { durationDays, trialingPlanId, trialIsFree } = config

    if (!durationDays || durationDays <= 0) {
      return <p className="text-sm">No trial configured.</p>
    }

    const trialPlanName = isCurrentPlan(trialingPlanId)
      ? 'current'
      : resolvePlanName(trialingPlanId!)
    const hasTrialingPlan = !isCurrentPlan(trialingPlanId)

    // Render based on plan type and trial configuration
    if (isFreePlan) {
      // Free plan: trial just gives access to features, no billing ever
      return (
        <div className="text-sm space-y-1">
          <p>
            Users get <span className="font-bold">{durationDays} days</span>
            {hasTrialingPlan && (
              <>
                {' '}
                with <span className="font-bold">&quot;{trialPlanName}&quot;</span> features
              </>
            )}
            , then continue on this free plan.
          </p>
          <p className="text-muted-foreground text-xs">No payment required.</p>
        </div>
      )
    } else {
      // Paid plan
      if (trialIsFree) {
        // Free trial on paid plan: no charge during trial, then billing starts
        return (
          <div className="text-sm space-y-1">
            <p>
              Users get <span className="font-bold">{durationDays} days free</span>
              {hasTrialingPlan && (
                <>
                  {' '}
                  with <span className="font-bold">&quot;{trialPlanName}&quot;</span> features
                </>
              )}
              , then billing starts on this plan.
            </p>
            <p className="text-muted-foreground text-xs">
              Payment method collected at checkout. If trial ends without payment, subscription
              requires checkout to continue.
            </p>
          </div>
        )
      } else {
        // Paid trial: charge from day 1, but with trialing plan features
        return (
          <div className="text-sm space-y-1">
            <p>
              Users are <span className="font-bold">charged from day 1</span>
              {hasTrialingPlan && (
                <>
                  , but get <span className="font-bold">&quot;{trialPlanName}&quot;</span> features
                  for <span className="font-bold">{durationDays} days</span>
                </>
              )}
              {!hasTrialingPlan && (
                <>
                  {' '}
                  with a <span className="font-bold">{durationDays}-day</span> introductory period
                </>
              )}
              .
            </p>
            <p className="text-muted-foreground text-xs">
              Payment collected immediately at checkout.
            </p>
          </div>
        )
      }
    }
  }

  return <div className="space-y-4">{renderTrialDescription()}</div>
}

interface EditableSpanProps {
  value?: string | number | null
  onChange: (newValue: string | number) => void
  options?: { name: string; value: string; label?: string }[]
  type?: 'text' | 'number'
  emptyMessage?: string
  quotes?: boolean
}

const EditableSpan = forwardRef(
  (
    { value, onChange, options, type = 'text', emptyMessage, quotes = false }: EditableSpanProps,
    _ref
  ) => {
    const [isEditing, setIsEditing] = useState(false)

    if (isEditing) {
      if (options) {
        return (
          <Select
            value={value as string}
            onValueChange={newValue => {
              onChange(newValue)
              setIsEditing(false)
            }}
            defaultOpen
            onOpenChange={open => !open && setIsEditing(false)}
          >
            <SelectTrigger className="w-[200px]  inline-flex" autoFocus>
              <SelectValue
                placeholder="Please select"
                onBlur={() => setIsEditing(false)}
                autoFocus
              />
            </SelectTrigger>
            <SelectContent>
              {options.map(option => (
                <SelectItem key={option.value} value={option.value}>
                  {option.name || option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )
      } else {
        return (
          <Input
            type={type}
            value={value as string | number}
            className="w-[200px] inline-flex"
            onChange={e =>
              onChange(type === 'number' ? parseInt(e.target.value, 10) : e.target.value)
            }
            onBlur={() => setIsEditing(false)}
            autoFocus
            min={type === 'number' ? 0 : undefined}
            max={type === 'number' ? 9999 : undefined}
            step={type === 'number' ? 1 : undefined}
          />
        )
      }
    }

    const msg = options
      ? options.find(o => o.value === value)?.name || emptyMessage || value
      : (value ?? emptyMessage)

    const shouldQuote = quotes && msg !== 'current' && msg !== emptyMessage

    return (
      <span onClick={() => setIsEditing(true)} className="font-bold cursor-pointer underline">
        {shouldQuote && <span>&quot;</span>}
        {msg ?? emptyMessage}
        {shouldQuote && <span>&quot;</span>}
      </span>
    )
  }
)
