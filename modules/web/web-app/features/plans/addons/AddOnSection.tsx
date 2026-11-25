import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { PlusIcon } from 'lucide-react'
import { useState } from 'react'
import { useSearchParams } from 'react-router'
import { toast } from 'sonner'

import { AddRow } from '@/features/plans/MoreForPlan'
import { AddOnPicker } from '@/features/plans/addons/AddOnPicker'
import { AddOnRow } from '@/features/plans/addons/AddOnRow'
import { hasTerm, orderCurrencies, pricedCurrencies } from '@/features/plans/currencies/priceCells'
import {
  useIsDraftVersion,
  usePlanOverview,
  usePlanWithVersion,
} from '@/features/plans/hooks/usePlan'
import { addOnPrices, offerChips } from '@/features/productCatalog/addons/addonWords'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  detachAddOnFromPlanVersion,
  listAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import type { AddOn } from '@/rpc/api/addons/v1/models_pb'

/**
 * The add-ons offered with this version, one row each as the plan's charges read; a draft offers
 * more at the end, from the catalog or new. A draft offering none opens on the catalog.
 */
export const AddOnSection = () => {
  const { version } = usePlanWithVersion()
  const overview = usePlanOverview()
  const isDraft = useIsDraftVersion()
  const queryClient = useQueryClient()
  const confirm = useConfirmationModal()
  const [params] = useSearchParams()
  const [adding, setAdding] = useState<boolean>()

  const addOns = useQuery(listAddOns, version ? { planVersionId: version.id } : skipToken).data
    ?.addOns
  const components =
    useQuery(listPriceComponents, version ? { planVersionId: version.id } : skipToken).data
      ?.components ?? []
  const versionCurrency = version?.currency ?? 'USD'
  // The plan editor's currency tab, and every currency the plan is priced in.
  const currency = params.get('currency') || versionCurrency
  const currencies = orderCurrencies(
    [versionCurrency, ...pricedCurrencies(components)],
    versionCurrency
  )
  const open = isDraft && (adding ?? addOns?.length === 0)

  const detach = useMutation(detachAddOnFromPlanVersion)
  const stopOffering = (addOn: AddOn) =>
    confirm(
      () => {
        if (!version) return
        detach
          .mutateAsync({ planVersionId: version.id, addOnId: addOn.id })
          .then(() =>
            queryClient.invalidateQueries({
              queryKey: createConnectQueryKey({ schema: listAddOns.parent, cardinality: undefined }),
            })
          )
          .catch(error =>
            toast.error(rpcErrorMessage(error, `${addOn.name} could not be removed from the plan`))
          )
      },
      {
        header: `Stop offering ${addOn.name} with this version?`,
        message: 'It stays in the catalog. Subscriptions that already have it keep it.',
      }
    )

  /** Where the plan is sold and the add-on has no price, besides the currency shown. */
  const gapsOf = (addOn: AddOn) => {
    const prices = addOnPrices(addOn)
    const sold = overview?.pricedOptions ?? []
    const labels = sold.length
      ? offerChips(prices, sold, overview?.options ?? [])
          .filter(chip => !chip.sold)
          .map(chip => chip.label)
      : currencies.filter(c => !prices.some(p => p.currency === c)).map(c => `${c}: no price`)
    return labels.filter(label => label !== `${currency}: no price`)
  }

  return (
    <div className="space-y-3">
      {addOns?.length ? (
        <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
          {addOns.map(addOn => (
            <AddOnRow
              key={addOn.id}
              addOn={addOn}
              currency={currency}
              gaps={gapsOf(addOn)}
              isDraft={isDraft}
              onRemove={() => stopOffering(addOn)}
            />
          ))}
        </div>
      ) : (
        addOns && !open && <p className="text-sm text-muted-foreground">No add-on is offered yet.</p>
      )}
      {open ? (
        <AddOnPicker
          offered={new Set(addOns?.map(a => a.id))}
          currency={currency}
          // A new add-on starts on the plan's billing period.
          cadence={components.flatMap(c => c.prices).find(hasTerm)?.cadence ?? BillingPeriod.MONTHLY}
          familyLocalId={overview?.productFamilyLocalId}
          onClose={() => setAdding(false)}
        />
      ) : (
        isDraft && (
          <AddRow onClick={() => setAdding(true)}>
            <PlusIcon size={12} /> Add an add-on
          </AddRow>
        )
      )}
    </div>
  )
}
