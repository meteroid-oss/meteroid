import { createConnectQueryKey, createQueryOptions, useMutation, useTransport } from '@connectrpc/connect-query'
import { Alert, AlertDescription, Button } from '@md/ui'
import { useQueries, useQueryClient } from '@tanstack/react-query'
import { AlertTriangle } from 'lucide-react'
import { toast } from 'sonner'

import { EntitlementRowsEditor } from '@/features/entitlements/rows/EntitlementRowsEditor'
import { useEditInDraft } from '@/features/plans/PlanActions'
import { useCreditPaidLabels } from '@/features/plans/creditPaidMetrics'
import { TypeSelect } from '@/features/resources/TypeSelect'
import { lower, useResourceTypes } from '@/features/resources/hooks'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { listEntitlementsByEntity } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { setPlanSoldFor } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

import type { Plan } from '@/rpc/api/plans/v1/models_pb'

/** Codes of the metrics `planVersionId` prices that a subscription for a `typeId` never bills. */
const usePricedMetricsNotAttributedTo = (planVersionId: string, typeId: string | undefined) => {
  const transport = useTransport()
  const components =
    useQuery(listPriceComponents, { planVersionId }, { enabled: !!typeId }).data?.components ?? []
  const metricIds = [
    ...new Set(
      components.flatMap(c => {
        const structure = c.feeStructure?.structure
        return structure?.case === 'usage' || structure?.case === 'capacity' ? [structure.value.metricId] : []
      })
    ),
  ]
  const metrics = useQueries({
    queries: metricIds.map(id => createQueryOptions(getBillableMetric, { id }, { transport })),
  })
  if (!typeId) return []
  return metrics.flatMap(r => {
    const metric = r.data?.billableMetric
    return metric && metric.attributedTo?.resourceTypeId !== typeId ? [metric.code] : []
  })
}

/** "Sold for" filters the wizard and drives warnings, never a block. */
export const usePlanSoldForWarnings = (plan: Pick<Plan, 'soldForResourceTypeId'>, planVersionId: string) => {
  const { byId } = useResourceTypes()
  const data = useQuery(listEntitlementsByEntity, { entity: { EntityId: { case: 'planVersionId', value: planVersionId } } }).data
  const notAttributed = usePricedMetricsNotAttributedTo(planVersionId, plan.soldForResourceTypeId)
  const type = plan.soldForResourceTypeId ? byId.get(plan.soldForResourceTypeId) : undefined
  if (!type) return []
  const customerLevel = (data?.features ?? []).filter(f => !f.levelResourceTypeId).map(f => f.name)
  return [
    ...(customerLevel.length
      ? [`Checks of ${customerLevel.join(', ')} without a ${lower(type.name)} return “Needs a ${lower(type.name)}”.`]
      : []),
    ...(notAttributed.length
      ? [
          `${notAttributed.join(', ')} ${notAttributed.length > 1 ? 'are' : 'is'} not attributed ` +
            `to ${lower(type.pluralName)}: never billed for a ${lower(type.name)}.`,
        ]
      : []),
  ]
}

/** A published version's own rows are fixed (edited through its draft); the plan's stay editable. */
export const PlanEntitlementsSection = ({
  plan,
  planVersionId,
  isDraft,
}: {
  plan: Plan
  planVersionId: string
  isDraft: boolean
}) => {
  const queryClient = useQueryClient()
  const { hasTypes } = useResourceTypes()
  const canManage = usePermissions().has('catalog:manage')
  const isExpress = useIsExpressOrganization()
  const editInDraft = useEditInDraft()
  const warnings = usePlanSoldForWarnings(plan, planVersionId)
  const creditPaid = useCreditPaidLabels(planVersionId)
  const soldFor = useMutation(setPlanSoldFor, {
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey({ schema: setPlanSoldFor.parent, cardinality: undefined }) }),
    onError: err => toast.error(err.message),
  })

  return (
    <section aria-labelledby="plan-entitlements" className="pb-8">
      <EntitlementRowsEditor
        owner={{ case: 'planVersionId', value: planVersionId }}
        soldForTypeId={plan.soldForResourceTypeId}
        ownerName={plan.name}
        metricLabels={creditPaid}
        canEdit={canManage && isDraft}
        canEditRow={row => canManage && (isDraft || row.entity?.EntityId.case === 'planId')}
        heading={
          <div className="flex flex-1 items-center justify-between gap-2">
            <h2 id="plan-entitlements" className="text-lg font-semibold">
              Entitlements
            </h2>
            {!isDraft && canManage && !isExpress && (
              <Button
                variant="outline"
                size="sm"
                disabled={editInDraft.pending}
                onClick={() => void editInDraft.open()}
              >
                {editInDraft.hasDraft ? 'Continue the draft' : 'Edit in a draft'}
              </Button>
            )}
          </div>
        }
      >
        {hasTypes && (
          <label className="flex max-w-xs flex-col gap-1 text-sm">
            Sold for
            <TypeSelect
              customerLabel="The customer"
              value={plan.soldForResourceTypeId}
              disabled={!canManage || soldFor.isPending}
              onChange={resourceTypeId => soldFor.mutate({ planId: plan.id, resourceTypeId })}
            />
          </label>
        )}
        {warnings.map(w => (
          <Alert key={w} variant="warning">
            <AlertTriangle className="h-4 w-4" />
            <AlertDescription>{w}</AlertDescription>
          </Alert>
        ))}
      </EntitlementRowsEditor>
    </section>
  )
}
