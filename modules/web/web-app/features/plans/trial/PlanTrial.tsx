import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Badge, Button, Form, SwitchFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { AlertTriangle } from 'lucide-react'
import { useController, useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { Slot, hasSlot } from '@/edition'
import { GrantChips, useGrantNames } from '@/features/offers/GrantChips'
import { GrantsField } from '@/features/offers/GrantsField'
import { grantChip, grantKinds, grantsFromProto } from '@/features/offers/grants'
import { toStartOfferInline } from '@/features/offers/offers'
import { ExtraActions } from '@/features/plans/MoreForPlan'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useZodForm } from '@/hooks/useZodForm'
import { env } from '@/lib/env'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  getOffer,
  getPlanStartOffer,
  listOffers,
  setPlanStartOffer,
} from '@/rpc/api/offers/v1/offers-OffersService_connectquery'
import { PlanTrial as PlanTrialModel, PlanType } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanOverview,
  getPlanWithVersion,
  listPlans,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

import { TrialTermsFields, useTrialPlans } from './TrialTermsFields'
import {
  ELIGIBILITY_LABELS,
  checkTrialTerms,
  toTrialForm,
  trialChips,
  trialTermsShape,
} from './trialTerms'

import type { TrialChip } from './trialTerms'
import type { GrantForm } from '@/features/offers/grants'
import type { PlanStartOffer as PlanStartOfferModel } from '@/rpc/api/offers/v1/offers_pb'

interface StartOfferProps {
  trial?: PlanTrialModel
  startOffer?: PlanStartOfferModel
  currentPlanId: string
  planName: string
  planType: PlanType
  /** The version's currency: credit and fixed discounts start in it. */
  currency?: string
  readOnly: boolean
  onDone: () => void
}

/** The plan's start offer, the same on every version: saved at once, for subscriptions to come. */
export const PlanStartOffer = (props: StartOfferProps) =>
  props.readOnly ? <StartOfferView {...props} /> : <StartOfferForm {...props} />

export const TrialChips = ({ chips }: { chips: TrialChip[] }) => (
  <div className="flex flex-wrap gap-1.5">
    {chips.map(chip => (
      <Badge key={chip.label} variant={chip.tone} size="sm">
        {chip.label}
      </Badge>
    ))}
  </div>
)

/** Refreshes what reads a plan's trial and start offer once it is saved. */
export const useRefreshPlanTrial = () => {
  const queryClient = useQueryClient()
  const invalidate = (queryKey: readonly unknown[]) => queryClient.invalidateQueries({ queryKey })
  return () =>
    Promise.all([
      invalidate(createConnectQueryKey({ schema: getPlanOverview.parent, cardinality: undefined })),
      invalidate(createConnectQueryKey({ schema: getPlanWithVersion, cardinality: undefined })),
      invalidate(createConnectQueryKey({ schema: listPlans, cardinality: undefined })),
      invalidate(createConnectQueryKey({ schema: getPlanStartOffer, cardinality: undefined })),
      invalidate(createConnectQueryKey({ schema: listOffers, cardinality: undefined })),
      invalidate(createConnectQueryKey({ schema: getOffer, cardinality: undefined })),
    ])
}

const EligibilityRow = ({ label }: { label: string }) => (
  <div className="flex items-center justify-between gap-3 text-[13px]">
    <span className="text-muted-foreground">Who can get it</span>
    <span>{label}</span>
  </div>
)

const CatalogPart = ({ trial, startOffer, currentPlanId, readOnly, onDone }: StartOfferProps) =>
  hasSlot('planDetail.trial') ? (
    <Slot
      name="planDetail.trial"
      planId={currentPlanId}
      trial={trial}
      readOnly={readOnly}
      onDone={onDone}
    />
  ) : startOffer?.offer ? (
    <EligibilityRow label={ELIGIBILITY_LABELS[startOffer.offer.eligibility]} />
  ) : null

/** Its own trial's terms: the start offer's, else the plan's. */
const termsOf = ({ trial, startOffer }: Pick<StartOfferProps, 'trial' | 'startOffer'>) =>
  startOffer ? startOffer.offer?.trial : trial?.terms

const StartOfferView = (props: StartOfferProps) => {
  const { currentPlanId, planName: ownName, planType, startOffer } = props
  const { planName } = useTrialPlans(currentPlanId)
  const terms = termsOf(props)
  const grants = grantsFromProto(startOffer?.offer?.grants ?? [])
  const names = useGrantNames(grants)
  return (
    <div className="space-y-4">
      {terms ? (
        <TrialChips
          chips={trialChips(toTrialForm(terms), {
            isFreePlan: planType === PlanType.FREE,
            planName,
            ownName,
          })}
        />
      ) : (
        <p className="text-sm text-muted-foreground">No trial</p>
      )}
      {grants.length > 0 && (
        <div className="space-y-1.5">
          <span className="text-xs text-muted-foreground">Also gives</span>
          <GrantChips chips={grants.map(grant => grantChip(grant, names))} />
        </div>
      )}
      <CatalogPart {...props} />
    </div>
  )
}

const startOfferSchema = z
  .object({
    ...trialTermsShape,
    hasTrial: z.boolean(),
    grants: z.array(z.custom<GrantForm>()),
  })
  .superRefine((value, ctx) => {
    if (value.hasTrial) checkTrialTerms(value, ctx)
  })

const StartOfferForm = (props: StartOfferProps) => {
  const { startOffer, currentPlanId, planName: ownName, planType, currency, onDone } = props
  const refresh = useRefreshPlanTrial()
  const isFreePlan = planType === PlanType.FREE
  const { planName } = useTrialPlans(currentPlanId)
  const enterprise = useEntitlement(entitlementCodes.SUBSCRIPTION_PHASES)
  const terms = termsOf(props)
  const offer = startOffer?.offer
  const methods = useZodForm({
    schema: startOfferSchema,
    defaultValues: {
      ...toTrialForm(terms),
      hasTrial: !!terms,
      grants: grantsFromProto(offer?.grants ?? []),
    },
    mode: 'onChange',
  })
  const values = useWatch({ control: methods.control })
  const grants = useController({ control: methods.control, name: 'grants' })
  const parsed = startOfferSchema.safeParse(values)
  const chips =
    parsed.success && parsed.data.hasTrial
      ? trialChips(parsed.data, { isFreePlan, planName, ownName })
      : undefined
  // A catalog offer, or one other plans name, is left as it is: saving gives this plan its own.
  const sharedWith = (startOffer?.planIds.length ?? 1) - 1
  const catalogOffer = offer?.standalone ? offer : undefined

  const mutation = useMutation(setPlanStartOffer, {
    onSuccess: async () => {
      await refresh()
      onDone()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The start offer could not be saved')),
  })

  return (
    <Form {...methods}>
      <form
        onSubmit={methods.handleSubmit(form =>
          mutation.mutate({
            planId: currentPlanId,
            change: {
              case: 'inline',
              value: toStartOfferInline(form.hasTrial ? form : undefined, form.grants, isFreePlan),
            },
          })
        )}
        className="flex min-h-0 flex-1 flex-col"
      >
        <div className="flex-1 space-y-6 overflow-y-auto pb-4">
          {(catalogOffer || sharedWith > 0) && (
            <div className="flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs">
              <AlertTriangle size={14} className="mt-px shrink-0 text-warning" />
              <span>
                {catalogOffer
                  ? `Catalog offer ${catalogOffer.name}: saving gives this plan its own`
                  : `Offer ${offer?.name}, shared with ${sharedWith} other ${
                      sharedWith === 1 ? 'plan' : 'plans'
                    }: saving gives this plan its own`}
              </span>
            </div>
          )}

          <div className="space-y-4">
            <SwitchFormField
              control={methods.control}
              name="hasTrial"
              label={isFreePlan ? 'Reverse trial' : 'Trial'}
            />
            {values.hasTrial && (
              <>
                {chips && <TrialChips chips={chips} />}
                <TrialTermsFields
                  control={methods.control}
                  isFreePlan={isFreePlan}
                  currentPlanId={currentPlanId}
                />
              </>
            )}
          </div>

          <div className="space-y-3">
            <h3 className="text-sm font-medium">Also gives</h3>
            <GrantsField
              value={grants.field.value}
              onChange={next => grants.field.onChange(next)}
              kinds={grantKinds({ addOns: true, features: env.entitlementsEnabled }).filter(
                kind => kind !== 'discount' || !isFreePlan
              )}
              lockedKinds={enterprise ? [] : ['addOn']}
              defaultCurrency={currency}
            />
          </div>

          <CatalogPart {...props} />
        </div>
        <div className="pb-4">
          <ExtraActions
            start={
              startOffer && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="text-destructive hover:text-destructive"
                  disabled={mutation.isPending}
                  onClick={() =>
                    mutation.mutate({
                      planId: currentPlanId,
                      change: { case: 'none', value: true },
                    })
                  }
                >
                  Remove start offer
                </Button>
              )
            }
          >
            <Button type="button" variant="secondary" size="sm" onClick={onDone}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" size="sm" disabled={mutation.isPending}>
              {startOffer ? 'Save' : 'Add start offer'}
            </Button>
          </ExtraActions>
        </div>
      </form>
    </Form>
  )
}
