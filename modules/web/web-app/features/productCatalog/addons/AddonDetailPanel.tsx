import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Input, Label, Skeleton, Textarea } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { PencilIcon, Trash2Icon } from 'lucide-react'
import { useId, useState } from 'react'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { LocalId } from '@/components/LocalId'
import { PanelHead, PanelSection } from '@/components/layouts'
import { EntityActivityTimeline } from '@/features/activity/EntityActivityTimeline'
import { simpleAmountOf, templateFor } from '@/features/plans/canvas/chargeFormat'
import { ChargeForm } from '@/features/plans/charges/ChargeForm'
import { ContextChip } from '@/features/plans/charges/InlineChargeFields'
import { cellPrices, hasTerm, termFormValue, termLabel } from '@/features/plans/currencies/priceCells'
import { useComponentMetric } from '@/features/plans/pricecomponents/componentRefs'
import { PricingDetailsView } from '@/features/plans/pricecomponents/components/PricingDetailsView'
import { buildPriceInputs } from '@/features/pricing/conversions'
import {
  buildDefaults,
  extractStructuralInfo,
  protoFeeTypeToComponentFeeType,
} from '@/features/pricing/formData'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  editAddOn,
  getAddOn,
  listAddOns,
  removeAddOn,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import { AddOnPricesGrid, AddOnPricesTable, inputOfPrice, priceOfInput, withPrice } from './AddOnPricesTable'
import { AddonEntitlementsSection } from './AddonEntitlementsSection'
import { AddonOfferedWith } from './AddonOfferedWith'
import { PerSubscriptionField, SelfServiceField } from './AddonSettingsFields'
import { addOnLine, addOnPrices } from './addonWords'

import type { StoredCharge } from '@/features/plans/charges/ChargeForm'
import type { ChargeLock } from '@/features/plans/charges/compile'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

type Cell = { currency: string; period: BillingPeriod }

/** What the panel changes at once: its name, how it is sold, its prices. */
type Patch = {
  name?: string
  description?: string
  selfServiceable?: boolean
  /** -1: any number (removes the cap). */
  maxInstancesPerSubscription?: number
}

/** The add-on open beside the catalog: what it is, its prices, the plans offering it. */
export const AddonDetailPanel = ({ addonId, onClose }: { addonId: string; onClose: () => void }) => {
  const queryClient = useQueryClient()
  const showConfirmationModal = useConfirmationModal()
  const canManage = usePermissions().has('catalog:manage')

  const addonQuery = useQuery(getAddOn, { addOnId: addonId })
  const addOn = addonQuery.data?.addOn
  // The grid's prices while a change saves, so a cell does not flash back.
  const [saving, setSaving] = useState<Price[]>()
  // Likewise how it is sold.
  const [pending, setPending] = useState<Patch>({})
  const [renaming, setRenaming] = useState(false)
  // The cell a single amount does not hold, priced in the charge form under the grid.
  const [editing, setEditing] = useState<Cell>()

  const invalidate = () =>
    Promise.all(
      [listAddOns, getAddOn].map(method =>
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: method.parent, cardinality: undefined }),
        })
      )
    )

  const removeMutation = useMutation(removeAddOn, {
    onSuccess: async () => {
      await invalidate()
      onClose()
    },
  })
  const editMutation = useMutation(editAddOn)

  /** Whether it saved. */
  const save = async (patch: Patch, failure: string) => {
    if (!addOn) return false
    setPending(patch)
    try {
      await editMutation.mutateAsync({ addOnId: addOn.id, ...patch })
      await invalidate()
      return true
    } catch (error) {
      toast.error(rpcErrorMessage(error, failure))
      return false
    } finally {
      setPending({})
    }
  }

  const changePrices = (next: Price[]) => {
    if (!addOn) return
    setSaving(next)
    editMutation
      .mutateAsync({ addOnId: addOn.id, prices: next.map(inputOfPrice) })
      .then(invalidate)
      .catch(error => toast.error(rpcErrorMessage(error, 'The price could not be saved')))
      .finally(() => setSaving(undefined))
  }

  if (addonQuery.isLoading || !addOn) {
    return (
      <div className="flex flex-col gap-3 p-4">
        <Skeleton className="h-6 w-48" />
        <Skeleton className="h-4 w-64" />
        <Skeleton className="h-24 w-full" />
      </div>
    )
  }

  const prices = saving ?? addOnPrices(addOn)
  const first = prices[0]
  const defaultCurrency = addOn.price?.currency ?? first?.currency
  const complex = first && simpleAmountOf(first) === undefined
  const shown = {
    selfServiceable: pending.selfServiceable ?? addOn.selfServiceable,
    max:
      pending.maxInstancesPerSubscription === undefined
        ? addOn.maxInstancesPerSubscription
        : pending.maxInstancesPerSubscription === -1
          ? undefined
          : pending.maxInstancesPerSubscription,
  }

  return (
    <div className="divide-y divide-border">
      <PanelHead
        title={
          <div className="flex items-center gap-2">
            <ProductIcon icon={addOn.productIcon} color={addOn.productColor} />
            <h2 className="truncate text-base font-semibold">{addOn.name}</h2>
          </div>
        }
        note={canManage ? 'Changes apply at once' : undefined}
        actions={
          <Gate perm="catalog:manage">
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              aria-label="Edit name and description"
              aria-pressed={renaming}
              onClick={() => setRenaming(!renaming)}
            >
              <PencilIcon size={14} />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="size-7 text-destructive hover:text-destructive"
              aria-label="Delete"
              onClick={() =>
                showConfirmationModal(() => removeMutation.mutate({ addOnId: addOn.id }))
              }
            >
              <Trash2Icon size={14} />
            </Button>
          </Gate>
        }
        onClose={onClose}
      />

      {renaming ? (
        <NameForm
          addOn={addOn}
          pending={editMutation.isPending}
          onCancel={() => setRenaming(false)}
          onSave={(name, description) =>
            void save({ name, description }, 'The add-on could not be renamed').then(
              saved => saved && setRenaming(false)
            )
          }
        />
      ) : (
        <div className="space-y-2 px-4 py-3">
          <p className="text-sm text-muted-foreground">{addOnLine(addOn)}</p>
          {addOn.description && <p className="text-sm">{addOn.description}</p>}
          <LocalId localId={addOn.id} className="max-w-24" />
        </div>
      )}

      <PanelSection
        title="Prices"
        hint="One price per currency and billing period. A subscription is offered the add-on in its own currency only."
      >
        {canManage ? (
          <AddOnPricesGrid
            prices={prices}
            onChange={changePrices}
            // A cell a single amount does not hold opens in the charge form below.
            onEditCell={(currency, period) => setEditing({ currency, period })}
            defaultCurrency={defaultCurrency}
          />
        ) : (
          <AddOnPricesTable prices={prices} defaultCurrency={defaultCurrency} />
        )}
        {editing && (
          <CellPriceForm
            key={`${editing.currency}:${editing.period}`}
            addOn={addOn}
            prices={prices}
            cell={editing}
            onCancel={() => setEditing(undefined)}
            onSave={next => {
              changePrices(next)
              setEditing(undefined)
            }}
          />
        )}
        {complex && defaultCurrency && !editing && (
          <PricingDetailsView prices={[first]} currency={defaultCurrency} />
        )}
      </PanelSection>

      {canManage && (
        <PanelSection title="How it is sold">
          <div className="space-y-2.5">
            <PerSubscriptionField
              value={shown.max}
              disabled={editMutation.isPending}
              onChange={max =>
                void save(
                  { maxInstancesPerSubscription: max ?? -1 },
                  'The add-on could not be changed'
                )
              }
            />
            <SelfServiceField
              value={shown.selfServiceable}
              disabled={editMutation.isPending}
              onChange={selfServiceable =>
                void save({ selfServiceable }, 'The add-on could not be changed')
              }
            />
          </div>
        </PanelSection>
      )}

      <PanelSection
        title="Offered with"
        hint="A subscription sees the add-on where it has a price in the subscription's currency, on the billing period of the option bought."
      >
        <AddonOfferedWith addOn={addOn} />
      </PanelSection>

      {env.entitlementsEnabled && (
        <div className="px-4 py-3">
          <AddonEntitlementsSection addonId={addOn.id} />
        </div>
      )}

      <PanelSection title="Activity">
        <EntityActivityTimeline
          entityType="add_on"
          entityId={addOn.id}
          emptyLabel="No activity yet for this add-on"
        />
      </PanelSection>
    </div>
  )
}

const NameForm = ({
  addOn,
  pending,
  onCancel,
  onSave,
}: {
  addOn: AddOn
  pending: boolean
  onCancel: () => void
  onSave: (name: string, description: string) => void
}) => {
  const descriptionId = useId()
  const [name, setName] = useState(addOn.name)
  const [description, setDescription] = useState(addOn.description ?? '')
  return (
    <form
      className="space-y-3 bg-muted/30 px-4 py-3"
      onSubmit={e => {
        e.preventDefault()
        if (name.trim()) onSave(name.trim(), description.trim())
      }}
      onKeyDown={e => e.key === 'Escape' && onCancel()}
    >
      <Input
        autoFocus
        aria-label="Name"
        value={name}
        onChange={e => setName(e.target.value)}
        className="h-9 text-base font-semibold"
      />
      <div className="space-y-1.5">
        <Label htmlFor={descriptionId} className="text-xs font-normal text-muted-foreground">
          Description
        </Label>
        <Textarea
          id={descriptionId}
          rows={2}
          placeholder="Optional, shown where customers pick it"
          value={description}
          onChange={e => setDescription(e.target.value)}
        />
      </div>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" size="sm" disabled={pending || !name.trim()}>
          Save
        </Button>
      </div>
    </form>
  )
}

/**
 * One cell's price in the charge form, its kind kept to the add-on's product. A cell without a
 * price starts from its closest one: its structure, amounts to re-enter.
 */
const CellPriceForm = ({
  addOn,
  prices,
  cell,
  onCancel,
  onSave,
}: {
  addOn: AddOn
  prices: Price[]
  cell: Cell
  onCancel: () => void
  onSave: (prices: Price[]) => void
}) => {
  const feeType = protoFeeTypeToComponentFeeType(addOn.feeType)
  const structural = extractStructuralInfo(feeType, addOn.feeStructure)
  const metric = useComponentMetric(structural.metricId)
  // Read once: the cell this form opened on.
  const [own] = useState(() => cellPrices(prices, cell.currency, cell.period)[0])
  const [initial] = useState(
    (): StoredCharge => ({
      feeType,
      formData: {
        ...buildDefaults(
          feeType,
          own ?? templateFor(prices, cell.currency, cell.period),
          false,
          structural.usageModel
        ),
        metricId: structural.metricId,
        slotUnitName: structural.slotUnitName,
        billingType: structural.billingType,
        term: termFormValue(cell.period),
      },
    })
  )
  const lock: ChargeLock = {
    feeType,
    billingType: structural.billingType as ChargeLock['billingType'],
  }
  const termed = own ? hasTerm(own) : prices[0] ? hasTerm(prices[0]) : true
  // The cell's price as it is now: the prices may have been saved since it opened.
  const current = cellPrices(prices, cell.currency, cell.period)[0]

  return (
    <div className="space-y-4 rounded-md border border-border bg-muted/30 p-3">
      <div className="flex items-center justify-between gap-3">
        <span className="truncate text-sm font-semibold text-foreground">
          {own ? 'Edit price' : 'Add price'}
        </span>
        <ContextChip>
          {termed ? `${cell.currency} · ${termLabel(cell.period)}` : cell.currency}
        </ContextChip>
      </div>
      <ChargeForm
        currency={cell.currency}
        cadence={cell.period}
        name={addOn.name}
        lock={lock}
        product={{
          metric,
          metricId: structural.metricId,
          slotUnitName: structural.slotUnitName,
        }}
        newProduct={false}
        initial={initial}
        submitLabel="Set price"
        remove={
          current && prices.length > 1
            ? {
                label: 'Remove this price',
                onClick: () => onSave(prices.filter(p => p !== current)),
              }
            : undefined
        }
        onCancel={onCancel}
        onSubmit={result => {
          if (result.included) return
          // One price per cell: capacity levels beyond the first are not sold on an add-on.
          const [input] = buildPriceInputs(result.pricingType, result.formData, cell.currency)
          if (!input) return
          const price = priceOfInput(input)
          // Its cell's billing period, whatever "Billed every" says.
          if (hasTerm(price)) price.cadence = cell.period
          onSave(withPrice(prices, price, current))
        }}
      />
    </div>
  )
}
