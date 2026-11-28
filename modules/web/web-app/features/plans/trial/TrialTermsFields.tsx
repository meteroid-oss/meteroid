import { InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { useWatch } from 'react-hook-form'

import { ChoiceCardsFormField } from '@/components/form'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useQuery } from '@/lib/connectrpc'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

import { THIS_PLAN } from './trialTerms'

import type { AtEnd, PlanNamer, TrialTermsForm, WithoutCard } from './trialTerms'
import type { ChoiceCardOption } from '@/components/form'
import type { Control, FieldValues } from 'react-hook-form'

interface PlanOption {
  value: string
  label: string
}

/** The plans a trial can give the features of, and switch to at its end (free ones in Core). */
export const useTrialPlans = (currentPlanId?: string) => {
  const plans = useQuery(listPlans, {}).data?.plans ?? []
  const paidSwitch = useEntitlement(entitlementCodes.SUBSCRIPTION_PHASES)
  const planName: PlanNamer = id =>
    id ? plans.find(p => p.id === id || p.activeVersion?.id === id)?.name : undefined
  const others = plans.filter(p => p.id !== currentPlanId)
  const featurePlans: PlanOption[] = others
    .filter(p => p.planType !== PlanType.FREE)
    .map(p => ({ value: p.id, label: p.name }))
  const switchPlans: PlanOption[] = others.flatMap(p =>
    p.activeVersion && (paidSwitch || p.free || p.planType === PlanType.FREE)
      ? [{ value: p.activeVersion.id, label: p.name }]
      : []
  )
  return { planName, featurePlans, switchPlans, paidSwitch }
}

const CARD_OPTIONS: ChoiceCardOption<'required' | 'optional'>[] = [
  { value: 'required', label: 'Required', hint: 'Checkout collects a card' },
  { value: 'optional', label: 'Optional', hint: 'Can start without a card' },
]

/** A saved switch to a version no plan has active any more stays choosable. */
const withSaved = (options: PlanOption[], saved: string | undefined, planName: PlanNamer) =>
  saved && !options.some(option => option.value === saved)
    ? [...options, { value: saved, label: planName(saved) ?? 'Saved plan version' }]
    : options

/** A trial's terms, bound to a form holding the `TrialTermsForm` fields at its root. */
export const TrialTermsFields = <T extends FieldValues>({
  control,
  isFreePlan,
  currentPlanId,
}: {
  control: Control<T>
  isFreePlan: boolean
  currentPlanId?: string
}) => {
  const c = control as unknown as Control<TrialTermsForm>
  const [atEnd, withoutCard, atEndPlan, withoutCardPlan] = useWatch({
    control: c,
    name: ['atEnd', 'withoutCard', 'atEndPlanVersionId', 'withoutCardPlanVersionId'],
  })
  const { planName, featurePlans, switchPlans, paidSwitch } = useTrialPlans(currentPlanId)
  const atEndPlans = withSaved(switchPlans, atEndPlan, planName)
  const withoutCardPlans = withSaved(switchPlans, withoutCardPlan, planName)

  const atEndOptions: ChoiceCardOption<AtEnd>[] = [
    { value: 'convert', label: 'Convert', hint: 'Billing starts' },
    { value: 'manual', label: 'Decide at the end', hint: 'Waits for the account owner' },
    { value: 'cancel', label: 'Cancel', hint: 'Ends with the trial' },
    {
      value: 'switch',
      label: 'Switch plan',
      hint: 'Choose a plan',
      disabled: atEndPlans.length === 0,
    },
  ]
  const withoutCardOptions: ChoiceCardOption<WithoutCard>[] = [
    { value: 'await_payment', label: 'Await payment', hint: 'Invoice issued, paused until paid' },
    { value: 'cancel', label: 'Cancel' },
    {
      value: 'switch',
      label: paidSwitch
        ? 'Switch plan'
        : switchPlans.length === 1
          ? `Switch to ${switchPlans[0].label}`
          : 'Switch to a free plan',
      disabled: withoutCardPlans.length === 0,
    },
  ]

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-2 gap-3">
        <InputFormField
          control={c}
          name="days"
          label="Length"
          type="number"
          inputMode="numeric"
          min={1}
          max={3650}
          step={1}
          rightText="days"
        />
        <SelectFormField control={c} name="featuresOf" label="Features of">
          <SelectItem value={THIS_PLAN}>This plan</SelectItem>
          {featurePlans.map(plan => (
            <SelectItem key={plan.value} value={plan.value}>
              {plan.label}
            </SelectItem>
          ))}
        </SelectFormField>
      </div>
      {!isFreePlan && (
        <>
          <ChoiceCardsFormField
            control={c}
            name="card"
            label="Card at sign-up"
            options={CARD_OPTIONS}
          />
          <div className="space-y-3">
            <ChoiceCardsFormField
              control={c}
              name="atEnd"
              label="At the end"
              options={atEndOptions}
            />
            {atEnd === 'switch' && (
              <PlanSelect control={c} name="atEndPlanVersionId" plans={atEndPlans} />
            )}
          </div>
          {atEnd === 'convert' && (
            <div className="space-y-3">
              <ChoiceCardsFormField
                control={c}
                name="withoutCard"
                label="If it can't be charged automatically"
                columns={3}
                options={withoutCardOptions}
              />
              {withoutCard === 'switch' && (
                <PlanSelect control={c} name="withoutCardPlanVersionId" plans={withoutCardPlans} />
              )}
            </div>
          )}
        </>
      )}
    </div>
  )
}

const PlanSelect = ({
  control,
  name,
  plans,
}: {
  control: Control<TrialTermsForm>
  name: 'atEndPlanVersionId' | 'withoutCardPlanVersionId'
  plans: PlanOption[]
}) => (
  <SelectFormField control={control} name={name} label="Switch to" placeholder="Choose a plan">
    {plans.map(plan => (
      <SelectItem key={plan.value} value={plan.value}>
        {plan.label}
      </SelectItem>
    ))}
  </SelectFormField>
)
