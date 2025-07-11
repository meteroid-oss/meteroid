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
import { forwardRef, useEffect, useState } from 'react'
import { Controller } from 'react-hook-form'
import { z } from 'zod'

import { useIsDraftVersion } from '@/features/plans/hooks/usePlan'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { PlanType, TrialConfig, TrialConfig_ActionAfterTrial } from '@/rpc/api/plans/v1/models_pb'
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
}

const mapActionAfterTrial = (action?: TrialConfig_ActionAfterTrial) => {
  switch (action) {
    case TrialConfig_ActionAfterTrial.BLOCK:
      return 'BLOCK'
    case TrialConfig_ActionAfterTrial.CHARGE:
      return 'CHARGE'
    case TrialConfig_ActionAfterTrial.DOWNGRADE:
      return 'DOWNGRADE'
    default:
      return 'BLOCK'
  }
}

const actionAfterTrialToGrpc = (action?: 'BLOCK' | 'CHARGE' | 'DOWNGRADE') => {
  switch (action) {
    case 'BLOCK':
      return TrialConfig_ActionAfterTrial.BLOCK
    case 'CHARGE':
      return TrialConfig_ActionAfterTrial.CHARGE
    case 'DOWNGRADE':
      return TrialConfig_ActionAfterTrial.DOWNGRADE
    case undefined:
      return TrialConfig_ActionAfterTrial.BLOCK
  }
}

export const PlanTrial = ({ config, currentPlanId, currentPlanVersionId }: TrialProps) => {
  const isDraft = useIsDraftVersion()
  const [isEditing, setIsEditing] = useState(false)

  if (isEditing) {
    return (
      <PlanTrialForm
        config={config}
        currentPlanId={currentPlanId}
        currentPlanVersionId={currentPlanVersionId}
        cancel={() => setIsEditing(false)}
        afterSubmit={() => setIsEditing(false)}
      />
    )
  }

  return (
    <div className="flex flex-row gap-2 items-center text-sm ">
      <PlanTrialReadonly config={config} currentPlanId={currentPlanId} />
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
  trialType: z.enum(['free', 'paid']).optional(),
  actionAfterTrial: z.enum(['BLOCK', 'CHARGE', 'DOWNGRADE']).optional(),
  downgradePlanId: z.union([z.string(), z.literal('current')]).optional(),
})

interface TrialConfigSentenceProps {
  config?: TrialConfig
  currentPlanId: string
  currentPlanVersionId: string
  afterSubmit: () => void
  cancel: () => void
}
export function PlanTrialForm({
  config,
  currentPlanId,
  currentPlanVersionId,
  afterSubmit,
  cancel,
}: TrialConfigSentenceProps) {
  console.log('config', config)
  const methods = useZodForm({
    schema: formSchema,
    defaultValues: {
      trialType: !config || config?.trialIsFree ? 'free' : 'paid',
      durationDays: config?.durationDays ?? 0,
      actionAfterTrial: mapActionAfterTrial(config?.actionAfterTrial),
      downgradePlanId: config?.downgradePlanId,
      trialingPlanId: config?.trialingPlanId,
      //   requiresPreAuthorization: config?.requiresPreAuthorization ?? false,
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
    // TODO filter out custom plans. We need to support arrays in the filter
    //   planTypeFilter
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

  const [trialingPlanId, trialType] = methods.watch(['trialingPlanId', 'trialType'])

  useEffect(() => {
    if (trialingPlanId === 'current' || trialingPlanId === currentPlanId) {
      methods.resetField('trialType')
    }
  }, [trialingPlanId, currentPlanId, methods.resetField])

  useEffect(() => {
    if (trialType === 'paid') {
      methods.resetField('downgradePlanId')
    }
  }, [trialType, currentPlanId, methods.resetField])

  return (
    <div className="space-y-4">
      <div>
        <Form {...methods}>
          <form
            onSubmit={methods.handleSubmit(data => {
              const trialIsFree = !data.trialType || data.trialType === 'free'

              const trial =
                data?.durationDays && data.durationDays > 0
                  ? {
                      actionAfterTrial: actionAfterTrialToGrpc(data.actionAfterTrial),
                      downgradePlanId:
                        trialIsFree && data.actionAfterTrial === 'DOWNGRADE'
                          ? data.downgradePlanId === 'current'
                            ? undefined
                            : data.downgradePlanId
                          : undefined,
                      trialingPlanId:
                        data.trialingPlanId === 'current' ? undefined : data.trialingPlanId,
                      durationDays: data.durationDays,
                      trialIsFree,
                      requiresPreAuthorization: false, // TODO
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
                    })
                  }
                >
                  Add trial
                </Button>
              </p>
            ) : (
              <p className="text-sm">
                Your users will get{' '}
                <Controller
                  name="durationDays"
                  control={methods.control}
                  render={({ field }) => <EditableSpan {...field} type="number" />}
                />{' '}
                days of the{' '}
                <Controller
                  name="trialingPlanId"
                  control={methods.control}
                  render={({ field }) => (
                    <EditableSpan
                      {...field}
                      options={nonFreePlansOptions}
                      emptyMessage="current"
                      quotes
                    />
                  )}
                />{' '}
                plan{' '}
                {!data.trialingPlanId ||
                data.trialingPlanId === 'current' ||
                data.trialingPlanId === currentPlanId ? (
                  'for free'
                ) : (
                  <Controller
                    name="trialType"
                    control={methods.control}
                    render={({ field }) => (
                      <EditableSpan
                        onChange={field.onChange}
                        value={field.value}
                        options={[
                          { value: 'free', name: 'for free' },
                          { value: 'paid', name: 'while paying for the current plan' },
                        ]}
                        emptyMessage="for free"
                      />
                    )}
                  />
                )}
                , then be{' '}
                {data.trialType === 'paid' ? (
                  'downgraded to the current plan'
                ) : (
                  <>
                    <Controller
                      name="actionAfterTrial"
                      control={methods.control}
                      render={({ field }) => (
                        <EditableSpan
                          {...field}
                          options={[
                            { value: 'BLOCK', name: 'blocked' },
                            { value: 'CHARGE', name: 'charged' },
                            { value: 'DOWNGRADE', name: 'downgraded' },
                          ]}
                          emptyMessage="blocked"
                        />
                      )}
                    />
                    {data.actionAfterTrial === 'DOWNGRADE' && (
                      <>
                        {' '}
                        to the{' '}
                        <Controller
                          name="downgradePlanId"
                          control={methods.control}
                          render={({ field }) => (
                            <EditableSpan
                              {...field}
                              options={plansOptions}
                              emptyMessage="current"
                              quotes
                            />
                          )}
                        />{' '}
                        plan
                      </>
                    )}
                  </>
                )}
                .
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
            )}

            {methods.formState.errors && (
              <div className="text-[0.8rem] font-medium text-destructive">
                {methods.formState.errors.durationDays && (
                  <p>
                    Invalid duration : {String(methods.formState.errors.trialingPlanId?.message)}
                  </p>
                )}
                {methods.formState.errors.actionAfterTrial && (
                  <p>
                    Invalid action : {String(methods.formState.errors.actionAfterTrial?.message)}
                  </p>
                )}
                {methods.formState.errors.downgradePlanId && (
                  <p>
                    Invalid downgrade plan :{' '}
                    {String(methods.formState.errors.downgradePlanId?.message)}
                  </p>
                )}
                {methods.formState.errors.trialingPlanId && (
                  <p>
                    Invalid trial plan : {String(methods.formState.errors.trialingPlanId?.message)}
                  </p>
                )}
                {methods.formState.errors.trialType && (
                  <p>Invalid trial type : {String(methods.formState.errors.trialType?.message)}</p>
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
}
export function PlanTrialReadonly({ config, currentPlanId }: PlanTrialReadonlyProps) {
  // TODO resolve plan names in server
  const { data: plansData, isLoading } = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const resolvePlanName = (id: string): string => {
    if (isLoading) return '...'
    const plan = plansData?.plans.find(p => p.id === id)
    return plan?.name ?? 'unknown'
  }

  const isCurrentPlan = (planId?: string) => !planId || planId === currentPlanId

  if (!config) {
    return <p className="text-sm">No trial configured.</p>
  }

  const renderActionAfterTrial = () => {
    const { actionAfterTrial, downgradePlanId } = config

    switch (actionAfterTrial) {
      case TrialConfig_ActionAfterTrial.DOWNGRADE:
        return (
          <>
            <span className="font-bold">downgraded</span> to the{' '}
            <span className="font-bold">
              {isCurrentPlan(downgradePlanId) ? 'current' : resolvePlanName(downgradePlanId!)}
            </span>{' '}
            plan
          </>
        )
      case TrialConfig_ActionAfterTrial.CHARGE:
        return <span className="font-bold">charged</span>
      default:
        return <span className="font-bold">blocked</span>
    }
  }

  const renderTrialDescription = () => {
    const { durationDays, trialingPlanId, trialIsFree } = config

    if (!durationDays || durationDays <= 0) {
      return <p className="text-sm">No trial configured.</p>
    }

    const trialPlanName = isCurrentPlan(trialingPlanId)
      ? 'current'
      : resolvePlanName(trialingPlanId!)
    const trialPriceDescription = trialIsFree ? 'for free' : 'while paying for the current plan'

    return (
      <p className="text-sm">
        Your users will get <span className="font-bold">{durationDays}</span> days of the{' '}
        <span className="font-bold">{trialPlanName}</span> plan {trialPriceDescription}, then be{' '}
        {trialIsFree ? renderActionAfterTrial() : 'downgraded to the current plan'}.
      </p>
    )
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
