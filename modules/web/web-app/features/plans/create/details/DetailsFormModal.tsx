import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
  SwitchFormField,
  TextareaFormField,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { FC, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { Slot, hasSlot, useModule } from '@/edition'
import { minorUnitsToInputAtPrecision, parseMinorUnitsAtPrecision } from '@/features/credit/amounts'
import {
  creditRuleKey,
  creditRuleSummary,
  defaultCreditRule,
  usageDenominations,
  versionChargesUsage,
} from '@/features/plans/creditRule'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { PlanLockChip } from '@/features/settings/components/PlanLockChip'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { editPlanSchema } from '@/lib/schemas/plans'
import { CreditExhaustedRule_Kind, Plan, PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanOverview,
  updateDraftPlanOverview,
  updatePublishedPlanOverview,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

export const DetailsFormModal: FC = () => {
  const navigate = useNavigate()
  const { version, plan } = usePlanWithVersion()

  return (
    <Dialog open onOpenChange={open => !open && navigate('..')}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit plan details</DialogTitle>
          <DialogDescription>
            {version?.isDraft
              ? 'Update the plan and the terms of its draft version.'
              : 'A published version keeps its terms: create a draft version to change them.'}
          </DialogDescription>
        </DialogHeader>
        {version && plan && <BasicDetailedForm version={version} plan={plan} />}
      </DialogContent>
    </Dialog>
  )
}

interface Props {
  plan: Plan
  version: PlanVersion
}

const BasicDetailedForm = ({ plan, version }: Props) => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const isDraft = useIsDraftVersion()
  const components =
    useQuery(listPriceComponents, { planVersionId: version.id }).data?.components ?? []
  const denominations = usageDenominations(components, version.currency, useCreditUnits())
  const chosen = version.creditExhaustedRule
  const defaultRule = defaultCreditRule(components)
  const licensingEnabled = useEntitlement(entitlementCodes.LICENSING)
  const hasLicenseIssuer = useModule('license-issuer') !== undefined

  const methods = useZodForm({
    schema: editPlanSchema,
    defaultValues: {
      description: plan?.description,
      planName: plan.name,
      netTerms: version.netTerms,
      taxInclusive: version.taxInclusive,
      selfServiceRank: plan?.selfServiceRank ?? undefined,
      creditRule: chosen ? creditRuleKey(chosen.kind) : defaultRule,
      creditCaps: {},
      issuesLicense: plan.issuesLicense,
    },
  })
  const ruleTouched = methods.formState.dirtyFields.creditRule
  const capsTouched = methods.formState.dirtyFields.creditCaps
  const storedCaps = JSON.stringify(
    Object.fromEntries(
      (chosen?.caps ?? []).flatMap(cap => {
        const denomination = denominations.find(d => d.code === cap.denomination)
        return denomination
          ? [[cap.denomination, minorUnitsToInputAtPrecision(cap.amount, denomination.precision)]]
          : []
      })
    )
  )
  // Caps read in their denomination's precision, once the credit units are loaded.
  useEffect(() => {
    if (!capsTouched) methods.resetField('creditCaps', { defaultValue: JSON.parse(storedCaps) })
  }, [storedCaps, capsTouched, methods])
  // Until chosen, the rule follows the default of the components as they load or change.
  useEffect(() => {
    if (!chosen && !ruleTouched) methods.resetField('creditRule', { defaultValue: defaultRule })
  }, [chosen, ruleTouched, defaultRule, methods])

  const updatePublishedPlan = useMutation(updatePublishedPlanOverview, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getPlanOverview.parent,
          cardinality: undefined,
        }),
      })
    },
  })

  const updateDraftPlan = useMutation(updateDraftPlanOverview, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getPlanOverview.parent,
          cardinality: undefined,
        }),
      })
    },
  })

  const isLoading = updatePublishedPlan.isPending || updateDraftPlan.isPending

  // Unset leaves the stored rule as it is: only an edited rule is sent.
  const creditRule = (data: z.infer<typeof editPlanSchema>) => {
    if (!ruleTouched && !capsTouched) return undefined
    const caps = []
    if (data.creditRule === 'INVOICE_UP_TO') {
      for (const denomination of denominations) {
        const input = data.creditCaps[denomination.code]?.trim()
        if (!input) continue
        const amount = parseMinorUnitsAtPrecision(input, denomination.precision)
        if (amount === null || amount <= 0n) {
          methods.setError(`creditCaps.${denomination.code}`, {
            message: `Enter a positive amount with at most ${denomination.precision} decimals`,
          })
          return null
        }
        caps.push({ denomination: denomination.code, amount })
      }
      if (caps.length === 0) {
        methods.setError('creditRule', { message: 'Set a cap' })
        return null
      }
    }
    return { kind: CreditExhaustedRule_Kind[data.creditRule], caps }
  }

  const onSubmit = async (data: z.infer<typeof editPlanSchema>) => {
    if (isDraft) {
      const rule = creditRule(data)
      if (rule === null) return
      await updateDraftPlan.mutateAsync({
        name: data.planName,
        description: data.description,
        netTerms: data.netTerms,
        taxInclusive: data.taxInclusive,
        planId: plan.id,
        planVersionId: version.id,
        currency: version.currency,
        creditExhaustedRule: rule,
      })
    } else {
      await updatePublishedPlan.mutateAsync({
        name: data.planName,
        description: data.description,
        planId: plan.id,
        planVersionId: version.id,
        selfServiceRank: data.selfServiceRank || 0,
        issuesLicense: data.issuesLicense !== plan.issuesLicense ? data.issuesLicense : undefined,
      })
    }

    navigate('..')
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <SettingsSection
          title="Plan"
          description="Applies to every version of this plan."
          divided={false}
        >
          <div className="space-y-4">
            <InputFormField
              name="planName"
              label="Name"
              control={methods.control}
              type="text"
              placeholder="Plan name"
            />
            <TextareaFormField
              name="description"
              label="Description"
              control={methods.control}
              placeholder="This plan gives access to ..."
            />
            {!isDraft && (
              <InputFormField
                name="selfServiceRank"
                label="Self-service rank"
                control={methods.control}
                type="number"
                placeholder="Not set"
                min={0}
                step={1}
              />
            )}
            {/* Without the license issuer the switch only lets an existing setting be turned off. */}
            {!isDraft && (hasLicenseIssuer || plan.issuesLicense) && (
              <div className="flex items-start gap-2">
                <SwitchFormField
                  name="issuesLicense"
                  control={methods.control}
                  label="Issues a license"
                  description="Each subscription to this plan gets a software license, revoked when the subscription ends."
                  disabled={!licensingEnabled && !plan.issuesLicense}
                />
                {hasLicenseIssuer && !licensingEnabled && <PlanLockChip className="mt-4" />}
              </div>
            )}
          </div>
        </SettingsSection>
        {isDraft && (
          <SettingsSection
            title={`Version ${version.version}`}
            description="Applies to this draft version only."
          >
            <div className="space-y-4">
              <InputFormField
                name="netTerms"
                control={methods.control}
                label="Net terms (days)"
                type="number"
                placeholder="30"
                min={0}
                max={180}
                step={1}
              />
              <SwitchFormField
                name="taxInclusive"
                control={methods.control}
                label="Prices include tax"
                description="Customers pay the listed price, with tax carved out of it on the invoice. Leave off to add tax on top."
              />
              {versionChargesUsage(components) &&
                (hasSlot('planEdit.creditRule') ? (
                  <Slot
                    name="planEdit.creditRule"
                    control={methods.control}
                    denominations={denominations}
                    defaulted={!chosen && !ruleTouched}
                  />
                ) : (
                  <div className="space-y-2">
                    <p className="text-sm font-medium leading-none">When credit runs out</p>
                    <p className="text-[0.8rem] text-muted-foreground">
                      {creditRuleSummary(chosen, components, denominations)}
                    </p>
                  </div>
                ))}
            </div>
          </SettingsSection>
        )}
        <DialogFooter className="mt-6">
          <Button type="button" variant="outline" onClick={() => navigate('..')}>
            Cancel
          </Button>
          <Button type="submit" disabled={isLoading}>
            {isLoading ? 'Saving…' : 'Save'}
          </Button>
        </DialogFooter>
      </form>
    </Form>
  )
}
