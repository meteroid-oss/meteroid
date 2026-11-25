import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Input, cn } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CheckIcon, PlusIcon, SearchIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { countsWords, pricesIn, rowWords } from '@/features/plans/canvas/billingWords'
import { FormFooter } from '@/features/plans/charges/InlineChargeFields'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { AddonCreateForm } from '@/features/productCatalog/addons/AddonCreateForm'
import { addOnPrices } from '@/features/productCatalog/addons/addonWords'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  attachAddOnToPlanVersion,
  listAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

import type { ChargeContext } from '@/features/plans/canvas/billingWords'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

type Props = {
  /** The add-ons the version offers already. */
  offered: Set<string>
  /** The currency the plan editor shows: each add-on's price in it. */
  currency: string
  /** The billing period a new add-on starts on: the plan's. */
  cadence: BillingPeriod
  /** Where a new add-on's product goes: the plan's product line. */
  familyLocalId?: string
  onClose: () => void
}

/** "Add an add-on": one of the catalog's, found by name, or a new one, offered once created. */
export const AddOnPicker = ({ offered, currency, cadence, familyLocalId, onClose }: Props) => {
  const { version } = usePlanWithVersion()
  const queryClient = useQueryClient()
  const [creating, setCreating] = useState(false)
  const [search, setSearch] = useState('')
  const query = useDebounceValue(search.trim(), 250)
  const catalog = useQuery(listAddOns, {
    search: query || undefined,
    pagination: { perPage: 20, page: 0 },
  })
  const addOns = catalog.data?.addOns ?? []
  const attach = useMutation(attachAddOnToPlanVersion)

  const offer = async (addOn: AddOn) => {
    if (!version) return
    try {
      await attach.mutateAsync({ planVersionId: version.id, addOnId: addOn.id })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listAddOns.parent, cardinality: undefined }),
      })
      onClose()
    } catch (error) {
      toast.error(rpcErrorMessage(error, `${addOn.name} could not be offered with this plan`))
    }
  }

  if (creating) {
    return (
      <div className="space-y-4 rounded-lg border border-border bg-muted/30 px-4 py-4">
        <h4 className="text-sm font-semibold text-foreground">New add-on</h4>
        <AddonCreateForm
          defaultCurrency={currency}
          defaultCadence={cadence}
          familyLocalId={familyLocalId}
          submitLabel="Create and offer"
          onCancel={() => setCreating(false)}
          // Not offered: back to the catalog, where it is now.
          onCreated={addOn => offer(addOn).finally(() => setCreating(false))}
        />
      </div>
    )
  }

  return (
    <div className="space-y-3 rounded-lg border border-border bg-muted/30 px-4 py-4">
      <div className="flex items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <SearchIcon
            size={14}
            className="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            autoFocus
            type="search"
            aria-label="Search the catalog's add-ons"
            placeholder="Search the catalog's add-ons"
            value={search}
            onChange={e => setSearch(e.target.value)}
            className="h-8 bg-background pl-8"
          />
        </div>
        <Button variant="outline" size="sm" hasIcon onClick={() => setCreating(true)}>
          <PlusIcon size={12} /> New add-on
        </Button>
      </div>

      {addOns.length > 0 ? (
        <ul className="divide-y divide-border overflow-hidden rounded-md border border-border bg-background">
          {addOns.map(addOn => (
            <li key={addOn.id}>
              <CatalogRow
                addOn={addOn}
                currency={currency}
                offered={offered.has(addOn.id)}
                disabled={attach.isPending}
                onOffer={() => void offer(addOn)}
              />
            </li>
          ))}
        </ul>
      ) : (
        !catalog.isLoading && (
          <p className="py-2 text-sm text-muted-foreground">
            {query ? `No add-on is named like “${query}”.` : 'The catalog has no add-on yet.'}{' '}
            <button
              type="button"
              className="text-foreground underline-offset-2 hover:underline"
              onClick={() => setCreating(true)}
            >
              Create one
            </button>
          </p>
        )
      )}

      <FormFooter>
        <Button type="button" variant="ghost" size="sm" onClick={onClose}>
          Cancel
        </Button>
      </FormFooter>
    </div>
  )
}

/** A catalog add-on in plan words: what it charges, its price in the plan's currency. */
const CatalogRow = ({
  addOn,
  currency,
  offered,
  disabled,
  onOffer,
}: {
  addOn: AddOn
  currency: string
  offered: boolean
  disabled: boolean
  onOffer: () => void
}) => {
  const prices = addOnPrices(addOn)
  const ctx: ChargeContext = { product: { feeStructure: addOn.feeStructure } }
  const words = rowWords(prices, currency, ctx)
  const first = pricesIn(prices, currency)[0] ?? prices[0]
  const what = [countsWords(first, ctx), words?.billed].filter(Boolean).join(' · ')

  return (
    <button
      type="button"
      disabled={offered || disabled}
      onClick={onOffer}
      className="flex w-full items-center gap-3 px-3 py-2.5 text-left transition-colors hover:bg-accent/50 disabled:cursor-default disabled:hover:bg-transparent"
    >
      <ProductIcon icon={addOn.productIcon} color={addOn.productColor} size="sm" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">{addOn.name}</span>
        <span className="block truncate text-xs text-muted-foreground">{what}</span>
      </span>
      <span
        className={cn(
          'shrink-0 text-right text-sm tabular-nums',
          words ? 'text-foreground' : 'text-xs text-warning'
        )}
      >
        {words?.amount ?? `No price in ${currency}`}
      </span>
      <span className="w-16 shrink-0 text-right text-xs font-medium text-muted-foreground">
        {offered ? (
          <span className="inline-flex items-center gap-1">
            <CheckIcon size={12} /> Offered
          </span>
        ) : (
          'Offer'
        )}
      </span>
    </button>
  )
}
