import {
  createConnectQueryKey,
  createQueryOptions,
  useMutation,
  useTransport,
} from '@connectrpc/connect-query'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
  cn,
} from '@md/ui'
import { useQueries, useQueryClient } from '@tanstack/react-query'
import { PlusIcon, XIcon } from 'lucide-react'
import { Link } from 'react-router'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  attachAddOnToPlanVersion,
  detachAddOnFromPlanVersion,
  listAddOns,
  listPlanVersionAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import {
  listPlanVersionById,
  listPlans,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import { addOnPrices, offerChips } from './addonWords'

import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { PlanOverview } from '@/rpc/api/plans/v1/models_pb'

/** The plans whose live version offers the add-on, and where a subscription to each sees it. */
export const AddonOfferedWith = ({ addOn }: { addOn: AddOn }) => {
  const transport = useTransport()
  const queryClient = useQueryClient()
  const basePath = useBasePath()
  const confirm = useConfirmationModal()
  const plansQuery = useQuery(listPlans, { pagination: { page: 0, perPage: 100 } })
  const live = (plansQuery.data?.plans ?? []).filter(p => p.activeVersion)
  const offers = useQueries({
    queries: live.map(p =>
      createQueryOptions(
        listPlanVersionAddOns,
        { planVersionId: p.activeVersion!.id },
        { transport }
      )
    ),
  })
  const loading = plansQuery.isPending || offers.some(q => q.isPending)
  const offering = live.filter((_, i) =>
    offers[i]?.data?.planVersionAddOns.some(link => link.addOnId === addOn.id)
  )
  const others = live.filter(p => !offering.includes(p))
  const prices = addOnPrices(addOn)

  const attach = useMutation(attachAddOnToPlanVersion)
  const detach = useMutation(detachAddOnFromPlanVersion)
  const refresh = () =>
    Promise.all(
      [listPlanVersionAddOns, listAddOns].map(method =>
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: method.parent, cardinality: undefined }),
        })
      )
    )
  // A draft in progress gets the change too, so publishing it does not undo it.
  const draftsOf = async (plan: PlanOverview) => {
    const versions = await queryClient
      .fetchQuery(createQueryOptions(listPlanVersionById, { planId: plan.id }, { transport }))
      .catch(() => undefined)
    return versions?.planVersions.filter(v => v.isDraft).map(v => v.id) ?? []
  }
  const offerWith = async (plan: PlanOverview) => {
    try {
      await attach.mutateAsync({ planVersionId: plan.activeVersion!.id, addOnId: addOn.id })
      const drafts = plan.hasDraftVersion ? await draftsOf(plan) : []
      await Promise.allSettled(
        drafts.map(id => attach.mutateAsync({ planVersionId: id, addOnId: addOn.id }))
      )
      toast.success(`${addOn.name} is offered with ${plan.name}`)
    } catch (error) {
      toast.error(rpcErrorMessage(error, `${addOn.name} could not be offered with ${plan.name}`))
    }
    await refresh()
  }
  const detachFrom = async (plan: PlanOverview) => {
    try {
      await detach.mutateAsync({ planVersionId: plan.activeVersion!.id, addOnId: addOn.id })
      const drafts = plan.hasDraftVersion ? await draftsOf(plan) : []
      await Promise.allSettled(
        drafts.map(id => detach.mutateAsync({ planVersionId: id, addOnId: addOn.id }))
      )
    } catch (error) {
      toast.error(rpcErrorMessage(error, 'The add-on could not be removed from the plan'))
    }
    await refresh()
  }
  const stopOffering = (plan: PlanOverview) =>
    confirm(
      () => void detachFrom(plan),
      {
        header: `Stop offering ${addOn.name} with ${plan.name}?`,
        message: 'Subscriptions that already have it keep it at its price.',
      }
    )

  if (loading) return <Skeleton className="h-10 w-full" />

  return (
    <div className="space-y-2">
      {offering.length === 0 && (
        <p className="text-sm text-muted-foreground">No plan offers it yet.</p>
      )}
      {offering.map(plan => {
        const chips = offerChips(prices, plan.pricedOptions, plan.options)
        return (
          <div
            key={plan.id}
            className="group/row flex items-start gap-3 rounded-md border border-border px-3 py-2"
          >
            <Link
              to={`${basePath}/plans/${plan.localId}`}
              className="w-28 shrink-0 truncate pt-0.5 text-sm font-medium hover:underline"
            >
              {plan.name}
            </Link>
            <div className="flex min-w-0 flex-1 flex-wrap gap-1.5">
              {chips.length === 0 && (
                <span className="text-xs text-muted-foreground">Not sold yet</span>
              )}
              {chips.map(chip => (
                <span
                  key={chip.label}
                  className={cn(
                    'rounded px-1.5 py-0.5 text-xs',
                    chip.sold
                      ? 'bg-success/15 text-success'
                      : 'border border-dashed border-warning/60 text-warning'
                  )}
                >
                  {chip.label}
                </span>
              ))}
            </div>
            <Gate perm="catalog:manage">
              <button
                type="button"
                aria-label={`Stop offering with ${plan.name}`}
                title="Stop offering with this plan"
                onClick={() => stopOffering(plan)}
                className="rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/row:opacity-100"
              >
                <XIcon size={13} />
              </button>
            </Gate>
          </div>
        )
      })}
      <Gate perm="catalog:manage">
        {others.length > 0 && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                hasIcon
                className="border-dashed"
                disabled={attach.isPending}
              >
                <PlusIcon size={12} /> Offer with a plan
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="max-h-64 overflow-y-auto">
              {others.map(plan => (
                <DropdownMenuItem key={plan.id} onSelect={() => void offerWith(plan)}>
                  {plan.name}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </Gate>
    </div>
  )
}
