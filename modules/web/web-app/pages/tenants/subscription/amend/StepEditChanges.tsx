import { create } from '@bufbuild/protobuf'
import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useAtom } from 'jotai'
import { Check, Minus, Pencil, Plus, Search, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useWizard } from 'react-use-wizard'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { termFromFormValue } from '@/features/plans/currencies/priceCells'
import { useComponentMetric } from '@/features/plans/pricecomponents/componentRefs'
import { priceSummaryBadges } from '@/features/plans/pricecomponents/utils'
import { formDataToPrice, protoPricingToFormData } from '@/features/pricing'
import { protoFeeTypeToComponentFeeType } from '@/features/pricing/formData'
import { levelAppliesNow, levelMove, levelWords } from '@/features/subscriptions/capacityLevels'
import {
  CustomPriceDialog,
  FeeDialog,
  usePlanVersionFamily,
} from '@/features/subscriptions/pricecomponents/ChargeDialogs'
import { useLevelUnit } from '@/features/subscriptions/useLevelUnit'
import {
  formatSubscriptionFee,
  formatSubscriptionFeeBillingPeriod,
  formatSubscriptionFeeCompact,
} from '@/features/subscriptions/utils/fees'
import { useQuery } from '@/lib/connectrpc'
import { priceToSubscriptionFee } from '@/lib/mapping/priceToSubscriptionFee'
import {
  EditAddOnDraft,
  EditComponentDraft,
  ExtraComponentDraft,
  amendSubscriptionAtom,
} from '@/pages/tenants/subscription/amend/state'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCreditUnits } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'
import {
  FeeStructure_BillingType,
  OneTimePricingSchema,
  SlotPricingSchema,
  UsagePricingSchema,
} from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import {
  SubscriptionAddOn,
  SubscriptionComponent,
  SubscriptionFee,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'
import {
  getSubscriptionDetails,
  listCapacityLevels,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { PlanChangeApplyMode } from '@/rpc/api/subscriptions/v1/subscriptions_pb'

import type { StoredCharge } from '@/features/plans/charges/ChargeForm'
import type { ChargeBodyContext } from '@/features/plans/charges/InlineChargeFields'
import type { ChargeLock } from '@/features/plans/charges/compile'
import type { ComponentFeeType } from '@/features/pricing'
import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type { OneTimePricing } from '@/rpc/api/prices/v1/models_pb'
import type { ComponentCapacityLevels } from '@/rpc/api/subscriptions/v1/subscriptions_pb'

/**
 * Map a live SubscriptionFee oneof to the ComponentFeeType used by the pricing
 * forms. The proto's `recurring` case corresponds to the `extraRecurring` fee type.
 */
const feeTypeFromSubscriptionFee = (fee?: SubscriptionFee): ComponentFeeType => {
  switch (fee?.fee.case) {
    case 'rate':
      return 'rate'
    case 'oneTime':
      return 'oneTime'
    case 'recurring':
      return 'extraRecurring'
    case 'capacity':
      return 'capacity'
    case 'slot':
      return 'slot'
    case 'usage':
      return 'usage'
    case 'credit':
      return 'credit'
    default:
      return 'rate'
  }
}

// A pack's subscription fee does not carry its expiry, so an override form would silently reset it.
export const canOverridePrice = (fee: SubscriptionFee | undefined): boolean =>
  fee?.fee.case !== 'credit'

const CREDIT_PACK_OVERRIDE_HINT =
  "A credit pack's price can't be overridden. Remove it and add a new credit pack component instead."

const OverridePriceButton = ({ fee, onClick }: { fee?: SubscriptionFee; onClick: () => void }) =>
  canOverridePrice(fee) ? (
    <Button variant="secondary" size="sm" onClick={onClick}>
      <Pencil className="h-4 w-4 mr-1" /> Override price
    </Button>
  ) : (
    <Tooltip>
      <TooltipTrigger asChild>
        <span tabIndex={0}>
          <Button variant="secondary" size="sm" disabled>
            <Pencil className="h-4 w-4 mr-1" /> Override price
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent multiline className="max-w-xs">{CREDIT_PACK_OVERRIDE_HINT}</TooltipContent>
    </Tooltip>
  )

const usageModelProtoToForm: Record<string, string> = {
  perUnit: 'per_unit',
  tiered: 'tiered',
  volume: 'volume',
  package: 'package',
  dimensionRates: 'dimension_rates',
  // Read by the server as rates by dimension; kept for data cached before.
  matrix: 'dimension_rates',
  rateCard: 'rate_card',
}

const periodToCadence = (
  period: SubscriptionFeeBillingPeriod
): 'MONTHLY' | 'QUARTERLY' | 'SEMIANNUAL' | 'ANNUAL' => {
  switch (period) {
    case SubscriptionFeeBillingPeriod.QUARTERLY:
      return 'QUARTERLY'
    case SubscriptionFeeBillingPeriod.SEMIANNUAL:
      return 'SEMIANNUAL'
    case SubscriptionFeeBillingPeriod.YEARLY:
      return 'ANNUAL'
    default:
      return 'MONTHLY'
  }
}

/** What a live fee lets its custom price be (its own kind), and its metric or seat label. */
const feeCharge = (fee?: SubscriptionFee): { lock: ChargeLock; product: ChargeBodyContext } => {
  const lock: ChargeLock = { feeType: feeTypeFromSubscriptionFee(fee) }
  switch (fee?.fee.case) {
    case 'capacity':
    case 'usage':
      return { lock, product: { metricId: fee.fee.value.metricId } }
    case 'slot':
      return { lock, product: { slotUnitName: fee.fee.value.unit } }
    case 'recurring':
      return {
        lock: {
          ...lock,
          billingType:
            fee.fee.value.billingType === FeeStructure_BillingType.ADVANCE ? 'ADVANCE' : 'ARREAR',
        },
        product: {},
      }
    default:
      return { lock, product: {} }
  }
}

/** A fixed quantity's price, its model kept: "3 × $10", or 3 units on tiers. */
const fixedQuantityFormData = (value: {
  rate: string
  quantity: number
  model?: OneTimePricing['model']
}) =>
  protoPricingToFormData({
    case: 'oneTimePricing',
    value: create(OneTimePricingSchema, {
      unitPrice: value.rate,
      quantity: value.quantity,
      model: value.model,
    }),
  })?.data ?? { unitPrice: value.rate, quantity: value.quantity }

/** Seed an override price form from a live fee, so its current prices show. */
export const subscriptionFeeToFormData = (
  fee: SubscriptionFee | undefined,
  period: SubscriptionFeeBillingPeriod
): Record<string, unknown> | undefined => {
  const f = fee?.fee
  const term = periodToCadence(period)
  switch (f?.case) {
    case 'rate':
      return { term, rate: f.value.rate }
    case 'oneTime':
      return fixedQuantityFormData(f.value)
    case 'recurring':
      return {
        term,
        billingType:
          f.value.billingType === FeeStructure_BillingType.ADVANCE ? 'ADVANCE' : 'ARREAR',
        ...fixedQuantityFormData(f.value),
      }
    case 'capacity':
      return {
        term,
        rate: f.value.rate,
        included: Number(f.value.included),
        overageRate: f.value.overageRate,
        creditUnitId: fee?.creditUnit?.creditUnitId ?? '',
        creditUnitRate: fee?.creditUnit?.rate ?? '',
      }
    case 'slot':
      return {
        term,
        slotUnitName: f.value.unit,
        quantityId: f.value.quantityId,
        unitRate: f.value.unitRate,
        minSlots: f.value.minSlots,
        maxSlots: f.value.maxSlots,
        // Seats priced by tiers or packages: the model's fields and `seatModel`.
        ...protoPricingToFormData({
          case: 'slotPricing',
          value: create(SlotPricingSchema, {
            unitRate: f.value.unitRate,
            minSlots: f.value.minSlots,
            maxSlots: f.value.maxSlots,
            model: f.value.model,
          }),
        })?.data,
        minimumCount: f.value.minSlots ?? 1,
        // Each policy enum currently has a single variant.
        upgradePolicy: 'PRORATED',
        downgradePolicy: 'REMOVE_AT_END_OF_PERIOD',
      }
    case 'usage': {
      const model = f.value.model
      const usageModel = usageModelProtoToForm[model.case ?? ''] ?? 'per_unit'
      const pricing = protoPricingToFormData({
        case: 'usagePricing',
        value: create(UsagePricingSchema, { model }),
      })
      return {
        metricId: f.value.metricId,
        usageModel,
        term,
        creditUnitId: fee?.creditUnit?.creditUnitId ?? '',
        creditUnitRate: fee?.creditUnit?.rate ?? '',
        ...pricing?.data,
      }
    }
    default:
      return undefined
  }
}

/** A fee's one-line summary; a credit pack also shows the credit it grants, in its unit. */
export const formatAmendFee = (
  fee: SubscriptionFee | undefined,
  currency: string,
  creditUnits?: CreditUnit[]
): string => {
  const compact = formatSubscriptionFeeCompact(fee, currency, creditUnits)
  return fee?.fee.case === 'credit'
    ? `${compact} · ${formatSubscriptionFee(fee, currency, creditUnits).details}`
    : compact
}

/**
 * Render a draft (form-data) price the same way live component fees are shown,
 * by converting form data -> Price -> SubscriptionFee. Falls back to the fee
 * type name if the form data can't be converted yet.
 */
const formatDraftFee = (
  feeType: ComponentFeeType,
  formData: Record<string, unknown>,
  currency: string,
  creditUnits?: CreditUnit[]
): string => {
  try {
    return formatAmendFee(
      priceToSubscriptionFee(formDataToPrice(feeType, formData, currency), { creditUnits }),
      currency,
      creditUnits
    )
  } catch {
    return feeType
  }
}

export const StepEditChanges = () => {
  const { nextStep } = useWizard()
  const [state, setState] = useAtom(amendSubscriptionAtom)
  const currency = state.currency || 'USD'

  const [overrideComponent, setOverrideComponent] = useState<SubscriptionComponent | null>(null)
  const [showAddComponent, setShowAddComponent] = useState(false)
  const [editExtraIndex, setEditExtraIndex] = useState<number | null>(null)
  const [showAddAddOn, setShowAddAddOn] = useState(false)
  const [overrideAddOn, setOverrideAddOn] = useState<SubscriptionAddOn | null>(null)

  const creditUnits = useQuery(listCreditUnits, { includeArchived: true }).data?.units
  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId: state.subscriptionId },
    { enabled: Boolean(state.subscriptionId) }
  )

  const components = subscriptionQuery.data?.priceComponents ?? []
  const addOns = subscriptionQuery.data?.addOns ?? []
  // Usage-levels charges sold at several levels: each moves to another level of its plan cell.
  const levelsQuery = useQuery(
    listCapacityLevels,
    { subscriptionId: state.subscriptionId },
    { enabled: Boolean(state.subscriptionId) }
  )
  const levelsById = useMemo(
    () =>
      new Map(
        (levelsQuery.data?.components ?? [])
          .filter(c => c.levels.length > 1)
          .map(c => [c.subscriptionComponentId, c])
      ),
    [levelsQuery.data]
  )
  const familyLocalId = usePlanVersionFamily(subscriptionQuery.data?.subscription?.planVersionId)
  // A new fee starts on the subscription's own cadence.
  const recurring = components.find(
    c => c.period !== SubscriptionFeeBillingPeriod.ONE_TIME && c.fee?.fee.case !== 'usage'
  )
  const cadence = recurring
    ? termFromFormValue(periodToCadence(recurring.period))
    : BillingPeriod.MONTHLY

  // Add-on catalog (full org catalog, no plan filter)
  const addOnsCatalogQuery = useQuery(listAddOns, {
    pagination: { perPage: 100, page: 0 },
  })
  const catalog = addOnsCatalogQuery.data?.addOns ?? []

  const catalogById = useMemo(
    () => new Map((addOnsCatalogQuery.data?.addOns ?? []).map(c => [c.id, c])),
    [addOnsCatalogQuery.data]
  )

  // Max number of instances of a catalog add-on allowed on a single subscription
  // (null = unlimited). Enforced client-side so it's blocked before preview/apply.
  const maxInstancesFor = (catalogAddOnId: string): number | null =>
    catalogById.get(catalogAddOnId)?.maxInstancesPerSubscription ?? null

  const editedById = useMemo(
    () => new Map(state.componentChanges.edited.map(e => [e.subscriptionComponentId, e])),
    [state.componentChanges.edited]
  )
  const removedComponentSet = useMemo(
    () => new Set(state.componentChanges.removedComponentIds),
    [state.componentChanges.removedComponentIds]
  )
  const editedAddOnById = useMemo(
    () => new Map(state.addOnChanges.edited.map(e => [e.subscriptionAddOnId, e])),
    [state.addOnChanges.edited]
  )
  const removedAddOnSet = useMemo(
    () => new Set(state.addOnChanges.removedAddOnIds),
    [state.addOnChanges.removedAddOnIds]
  )

  // --- Component actions ---

  const saveOverride = (draft: EditComponentDraft) => {
    setState(prev => ({
      ...prev,
      componentChanges: {
        ...prev.componentChanges,
        edited: [
          ...prev.componentChanges.edited.filter(
            e => e.subscriptionComponentId !== draft.subscriptionComponentId
          ),
          draft,
        ],
      },
    }))
    setOverrideComponent(null)
  }

  // A higher level applies at once, a lower one at the period's end (R-135): the mode follows.
  const pickLevel = (
    component: SubscriptionComponent,
    cell: ComponentCapacityLevels,
    priceId: string
  ) => {
    const level = cell.levels.find(l => l.priceId === priceId)
    if (!level) return
    if (level.current) return clearOverride(component.id)
    saveOverride({
      subscriptionComponentId: component.id,
      name: component.name,
      feeType: 'capacity',
      formData: {
        ...subscriptionFeeToFormData(component.fee, component.period),
        rate: level.rate,
        included: Number(level.included),
        overageRate: level.overageRate,
      },
      level: {
        priceId: level.priceId,
        included: level.included,
        rate: level.rate,
        fromIncluded: cell.included,
      },
    })
    setState(prev => ({
      ...prev,
      applyMode: levelAppliesNow(cell.included, level.included)
        ? PlanChangeApplyMode.IMMEDIATE
        : PlanChangeApplyMode.END_OF_PERIOD,
    }))
  }

  const clearOverride = (id: string) => {
    setState(prev => ({
      ...prev,
      componentChanges: {
        ...prev.componentChanges,
        edited: prev.componentChanges.edited.filter(e => e.subscriptionComponentId !== id),
      },
    }))
  }

  const toggleRemoveComponent = (id: string) => {
    setState(prev => {
      const isRemoved = prev.componentChanges.removedComponentIds.includes(id)
      return {
        ...prev,
        componentChanges: {
          ...prev.componentChanges,
          removedComponentIds: isRemoved
            ? prev.componentChanges.removedComponentIds.filter(x => x !== id)
            : [...prev.componentChanges.removedComponentIds, id],
        },
      }
    })
  }

  const addExtraComponent = (component: ExtraComponentDraft) => {
    setState(prev => ({
      ...prev,
      componentChanges: {
        ...prev.componentChanges,
        added: [...prev.componentChanges.added, component],
      },
    }))
    setShowAddComponent(false)
  }

  const removeExtraComponent = (index: number) => {
    setState(prev => ({
      ...prev,
      componentChanges: {
        ...prev.componentChanges,
        added: prev.componentChanges.added.filter((_, i) => i !== index),
      },
    }))
  }

  const updateExtraComponent = (index: number, draft: ExtraComponentDraft) => {
    setState(prev => ({
      ...prev,
      componentChanges: {
        ...prev.componentChanges,
        added: prev.componentChanges.added.map((c, i) => (i === index ? draft : c)),
      },
    }))
    setEditExtraIndex(null)
  }

  // --- Add-on actions ---

  // Merge a partial change into the single edit entry for an add-on, dropping
  // the entry entirely when neither a quantity nor a price override remains.
  const upsertAddOnEdit = (id: string, name: string, patch: Partial<EditAddOnDraft>) => {
    setState(prev => {
      const existing = prev.addOnChanges.edited.find(e => e.subscriptionAddOnId === id)
      const merged: EditAddOnDraft = {
        subscriptionAddOnId: id,
        name,
        quantity: existing?.quantity,
        priceOverride: existing?.priceOverride,
        ...patch,
      }
      const isNoop = merged.quantity == null && !merged.priceOverride
      return {
        ...prev,
        addOnChanges: {
          ...prev.addOnChanges,
          edited: [
            ...prev.addOnChanges.edited.filter(e => e.subscriptionAddOnId !== id),
            ...(isNoop ? [] : [merged]),
          ],
        },
      }
    })
  }

  const setAddOnQuantity = (
    addOn: { id: string; name: string; addOnId: string; quantity: number },
    quantity: number
  ) => {
    if (quantity < 1) return
    const max = maxInstancesFor(addOn.addOnId)
    if (max != null && quantity > max) return
    // Resetting to the original quantity is not a change — drop the quantity edit
    // so an unchanged add-on never triggers a zero-sum proration preview.
    upsertAddOnEdit(addOn.id, addOn.name, {
      quantity: quantity === addOn.quantity ? undefined : quantity,
    })
  }

  const saveAddOnOverride = (
    addOn: { id: string; name: string },
    priceOverride: NonNullable<EditAddOnDraft['priceOverride']>
  ) => {
    upsertAddOnEdit(addOn.id, addOn.name, { priceOverride })
    setOverrideAddOn(null)
  }

  const clearAddOnOverride = (addOn: { id: string; name: string }) =>
    upsertAddOnEdit(addOn.id, addOn.name, { priceOverride: undefined })

  const toggleRemoveAddOn = (id: string) => {
    setState(prev => {
      const isRemoved = prev.addOnChanges.removedAddOnIds.includes(id)
      return {
        ...prev,
        addOnChanges: {
          ...prev.addOnChanges,
          removedAddOnIds: isRemoved
            ? prev.addOnChanges.removedAddOnIds.filter(x => x !== id)
            : [...prev.addOnChanges.removedAddOnIds, id],
        },
      }
    })
  }

  const addNewAddOn = (addOnId: string) => {
    setState(prev => {
      if (prev.addOnChanges.added.some(a => a.addOnId === addOnId)) return prev
      return {
        ...prev,
        addOnChanges: {
          ...prev.addOnChanges,
          added: [...prev.addOnChanges.added, { addOnId, quantity: 1 }],
        },
      }
    })
  }

  const removeNewAddOn = (addOnId: string) => {
    setState(prev => ({
      ...prev,
      addOnChanges: {
        ...prev.addOnChanges,
        added: prev.addOnChanges.added.filter(a => a.addOnId !== addOnId),
      },
    }))
  }

  const setNewAddOnQuantity = (addOnId: string, quantity: number) => {
    if (quantity < 1) return
    const max = maxInstancesFor(addOnId)
    if (max != null && quantity > max) return
    setState(prev => ({
      ...prev,
      addOnChanges: {
        ...prev.addOnChanges,
        added: prev.addOnChanges.added.map(a => (a.addOnId === addOnId ? { ...a, quantity } : a)),
      },
    }))
  }

  return (
    <div className="space-y-6">
      <PageSection
        header={{
          title: 'Edit subscription',
          subtitle: 'Override prices, add or remove components and add-ons',
        }}
      >
        <div className="space-y-8">
          {/* --- Components --- */}
          <div>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-sm font-medium">Components</h3>
              <Button variant="secondary" size="sm" onClick={() => setShowAddComponent(true)}>
                <Plus className="h-4 w-4 mr-1" /> Add component
              </Button>
            </div>
            <div className="bg-card rounded-lg border border-border divide-y divide-border">
              {components.length === 0 && (
                <div className="p-4 text-sm text-muted-foreground">No components.</div>
              )}
              {components.map(component => {
                const edited = editedById.get(component.id)
                const isRemoved = removedComponentSet.has(component.id)
                const cell = levelsById.get(component.id)
                return (
                  <div
                    key={component.id}
                    className={`flex items-center justify-between gap-3 p-3 ${isRemoved ? 'bg-destructive/5' : ''}`}
                  >
                    <div className="min-w-0">
                      <div
                        className={`text-sm font-medium ${isRemoved ? 'line-through text-muted-foreground' : ''}`}
                      >
                        {edited?.name || component.name}
                        {edited?.level ? (
                          <LevelBadge
                            fee={component.fee}
                            currency={currency}
                            from={edited.level.fromIncluded}
                            to={edited.level.included}
                          />
                        ) : (
                          edited && (
                            <Badge
                              variant="outline"
                              size="sm"
                              className="ml-2 text-brand border-brand/30"
                            >
                              Price overridden
                            </Badge>
                          )
                        )}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        {edited ? (
                          <>
                            <span className="line-through opacity-60">
                              {formatAmendFee(component.fee, currency, creditUnits)}
                            </span>{' '}
                            <span className="text-brand">
                              {formatDraftFee(
                                edited.feeType,
                                edited.formData,
                                currency,
                                creditUnits
                              )}
                            </span>
                          </>
                        ) : (
                          formatAmendFee(component.fee, currency, creditUnits)
                        )}
                        <span className="opacity-70">
                          {' · '}
                          {formatSubscriptionFeeBillingPeriod(component.period)}
                        </span>
                      </div>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {edited && !isRemoved && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => clearOverride(component.id)}
                        >
                          Reset
                        </Button>
                      )}
                      {cell && !isRemoved && (
                        <LevelPicker
                          cell={cell}
                          fee={component.fee}
                          period={component.period}
                          currency={currency}
                          value={
                            edited?.level?.priceId ??
                            (edited ? '' : (cell.levels.find(l => l.current)?.priceId ?? ''))
                          }
                          onPick={priceId => pickLevel(component, cell, priceId)}
                        />
                      )}
                      {!isRemoved && (
                        <OverridePriceButton
                          fee={component.fee}
                          onClick={() => setOverrideComponent(component)}
                        />
                      )}
                      <Button
                        variant={isRemoved ? 'secondary' : 'ghost'}
                        size="sm"
                        onClick={() => toggleRemoveComponent(component.id)}
                      >
                        {isRemoved ? 'Undo remove' : <Trash2 className="h-4 w-4" />}
                      </Button>
                    </div>
                  </div>
                )
              })}

              {/* Added ad-hoc components */}
              {state.componentChanges.added.map((extra, index) => (
                <div
                  key={`extra-${index}`}
                  className="flex items-center justify-between gap-3 p-3 bg-brand/5"
                >
                  <div className="min-w-0">
                    <div className="text-sm font-medium">
                      {extra.name}
                      <Badge
                        variant="outline"
                        size="sm"
                        className="ml-2 text-brand border-brand/30"
                      >
                        Added
                      </Badge>
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {formatDraftFee(extra.feeType, extra.formData, currency, creditUnits)}
                    </div>
                  </div>
                  <div className="flex items-center gap-2 shrink-0">
                    <Button variant="secondary" size="sm" onClick={() => setEditExtraIndex(index)}>
                      <Pencil className="h-4 w-4 mr-1" /> Edit
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => removeExtraComponent(index)}>
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* --- Add-ons --- */}
          <div>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-sm font-medium">Add-ons</h3>
              <Button variant="secondary" size="sm" onClick={() => setShowAddAddOn(true)}>
                <Plus className="h-4 w-4 mr-1" /> Add add-on
              </Button>
            </div>
            <div className="bg-card rounded-lg border border-border divide-y divide-border">
              {addOns.length === 0 && state.addOnChanges.added.length === 0 && (
                <div className="p-4 text-sm text-muted-foreground">No add-ons attached.</div>
              )}
              {addOns.map(addOn => {
                const edited = editedAddOnById.get(addOn.id)
                const isRemoved = removedAddOnSet.has(addOn.id)
                const quantity = edited?.quantity ?? addOn.quantity
                const max = maxInstancesFor(addOn.addOnId)
                return (
                  <div
                    key={addOn.id}
                    className={`flex items-center justify-between gap-3 p-3 ${isRemoved ? 'bg-destructive/5' : ''}`}
                  >
                    <div className="min-w-0">
                      <div
                        className={`text-sm font-medium ${isRemoved ? 'line-through text-muted-foreground' : ''}`}
                      >
                        {addOn.name}
                        {edited?.quantity != null && edited.quantity !== addOn.quantity && (
                          <Badge
                            variant="outline"
                            size="sm"
                            className="ml-2 text-brand border-brand/30"
                          >
                            Qty {addOn.quantity} → {edited.quantity}
                          </Badge>
                        )}
                        {edited?.priceOverride && (
                          <Badge
                            variant="outline"
                            size="sm"
                            className="ml-2 text-brand border-brand/30"
                          >
                            Price overridden
                          </Badge>
                        )}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        {edited?.priceOverride ? (
                          <>
                            <span className="line-through opacity-60">
                              {formatAmendFee(addOn.fee, currency, creditUnits)}
                            </span>{' '}
                            <span className="text-brand">
                              {formatDraftFee(
                                edited.priceOverride.feeType,
                                edited.priceOverride.formData,
                                currency,
                                creditUnits
                              )}
                            </span>
                          </>
                        ) : (
                          formatAmendFee(addOn.fee, currency, creditUnits)
                        )}
                        <span className="opacity-70">
                          {' · '}
                          {formatSubscriptionFeeBillingPeriod(addOn.period)}
                        </span>
                      </div>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {!isRemoved && (
                        <div className="flex items-center gap-1">
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 w-7 p-0"
                            onClick={() => setAddOnQuantity(addOn, quantity - 1)}
                            disabled={quantity <= 1}
                          >
                            <Minus className="h-3 w-3" />
                          </Button>
                          <span className="text-sm w-6 text-center">{quantity}</span>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 w-7 p-0"
                            onClick={() => setAddOnQuantity(addOn, quantity + 1)}
                            disabled={max != null && quantity >= max}
                          >
                            <Plus className="h-3 w-3" />
                          </Button>
                        </div>
                      )}
                      {edited?.priceOverride && !isRemoved && (
                        <Button variant="ghost" size="sm" onClick={() => clearAddOnOverride(addOn)}>
                          Reset
                        </Button>
                      )}
                      {!isRemoved && (
                        <OverridePriceButton
                          fee={addOn.fee}
                          onClick={() => setOverrideAddOn(addOn)}
                        />
                      )}
                      <Button
                        variant={isRemoved ? 'secondary' : 'ghost'}
                        size="sm"
                        onClick={() => toggleRemoveAddOn(addOn.id)}
                      >
                        {isRemoved ? 'Undo remove' : <Trash2 className="h-4 w-4" />}
                      </Button>
                    </div>
                  </div>
                )
              })}

              {/* Newly added add-ons */}
              {state.addOnChanges.added.map(added => {
                const cat = catalog.find(c => c.id === added.addOnId)
                const priceLabel = cat
                  ? priceSummaryBadges(
                      protoFeeTypeToComponentFeeType(cat.feeType),
                      cat.price,
                      currency,
                      creditUnits
                    ).join(' / ')
                  : undefined
                return (
                  <div
                    key={`new-${added.addOnId}`}
                    className="flex items-center justify-between gap-3 p-3 bg-brand/5"
                  >
                    <div className="min-w-0">
                      <div className="text-sm font-medium">
                        {cat?.name ?? added.addOnId}
                        <Badge
                          variant="outline"
                          size="sm"
                          className="ml-2 text-brand border-brand/30"
                        >
                          Added
                        </Badge>
                      </div>
                      {priceLabel && (
                        <div className="text-xs text-muted-foreground">{priceLabel}</div>
                      )}
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      <div className="flex items-center gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-7 w-7 p-0"
                          onClick={() => setNewAddOnQuantity(added.addOnId, added.quantity - 1)}
                          disabled={added.quantity <= 1}
                        >
                          <Minus className="h-3 w-3" />
                        </Button>
                        <span className="text-sm w-6 text-center">{added.quantity}</span>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-7 w-7 p-0"
                          onClick={() => setNewAddOnQuantity(added.addOnId, added.quantity + 1)}
                          disabled={
                            maxInstancesFor(added.addOnId) != null &&
                            added.quantity >= maxInstancesFor(added.addOnId)!
                          }
                        >
                          <Plus className="h-3 w-3" />
                        </Button>
                      </div>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => removeNewAddOn(added.addOnId)}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </div>
                )
              })}
            </div>
          </div>
        </div>
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="primary" onClick={nextStep}>
          Review changes
        </Button>
      </div>

      {/* Override component price modal */}
      {overrideComponent && (
        <OverridePriceModal
          component={overrideComponent}
          currency={currency}
          onClose={() => setOverrideComponent(null)}
          onSave={saveOverride}
        />
      )}

      {/* Override add-on price modal */}
      {overrideAddOn && (
        <OverrideAddOnPriceModal
          addOn={overrideAddOn}
          currency={currency}
          onClose={() => setOverrideAddOn(null)}
          onSave={priceOverride => saveAddOnOverride(overrideAddOn, priceOverride)}
        />
      )}

      {/* Add ad-hoc component */}
      {showAddComponent && (
        <FeeDialog
          currency={currency}
          cadence={cadence}
          familyLocalId={familyLocalId}
          onClose={() => setShowAddComponent(false)}
          onSave={addExtraComponent}
        />
      )}

      {/* Edit an added ad-hoc component */}
      {editExtraIndex !== null && state.componentChanges.added[editExtraIndex] && (
        <FeeDialog
          fee={state.componentChanges.added[editExtraIndex]}
          currency={currency}
          cadence={cadence}
          familyLocalId={familyLocalId}
          onClose={() => setEditExtraIndex(null)}
          onSave={draft => updateExtraComponent(editExtraIndex, draft)}
        />
      )}

      {/* Add add-on picker */}
      {showAddAddOn && (
        <AddAddOnModal
          catalog={catalog}
          selectedIds={state.addOnChanges.added.map(a => a.addOnId)}
          onClose={() => setShowAddAddOn(false)}
          onAdd={addNewAddOn}
          onRemove={removeNewAddOn}
        />
      )}
    </div>
  )
}

// --- Usage levels: another level of the plan, in words ---

const LevelPicker = ({
  cell,
  fee,
  period,
  currency,
  value,
  onPick,
}: {
  cell: ComponentCapacityLevels
  fee?: SubscriptionFee
  period: SubscriptionFeeBillingPeriod
  currency: string
  value: string
  onPick: (priceId: string) => void
}) => {
  const unit = useLevelUnit(fee, currency)
  return (
    <Select value={value} onValueChange={onPick}>
      <SelectTrigger className="h-8 w-auto text-xs" aria-label="Level">
        <SelectValue placeholder="Change level" />
      </SelectTrigger>
      <SelectContent>
        {cell.levels.map(level => (
          <SelectItem key={level.priceId} value={level.priceId}>
            {levelWords(level, currency, period, unit)}
            {level.current ? ' · current' : ''}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

const LevelBadge = ({
  fee,
  currency,
  from,
  to,
}: {
  fee?: SubscriptionFee
  currency: string
  from: bigint
  to: bigint
}) => {
  const unit = useLevelUnit(fee, currency)
  return (
    <Badge variant="outline" size="sm" className="ml-2 text-brand border-brand/30">
      Level {levelMove(from, to, currency, unit)}
    </Badge>
  )
}

// --- Custom prices: the plan editor's form, the fee's kind kept ---

/** A live fee's custom price, starting from the fee as billed today. */
const FeePriceDialog = ({
  name,
  fee,
  period,
  currency,
  onClose,
  onSave,
}: {
  name: string
  fee?: SubscriptionFee
  period: SubscriptionFeeBillingPeriod
  currency: string
  onClose: () => void
  onSave: (charge: StoredCharge) => void
}) => {
  const { lock, product } = feeCharge(fee)
  const metric = useComponentMetric(product.metricId)
  return (
    <CustomPriceDialog
      name={name}
      currency={currency}
      cadence={termFromFormValue(periodToCadence(period))}
      lock={lock}
      product={{ ...product, metric }}
      initial={{ feeType: lock.feeType, formData: subscriptionFeeToFormData(fee, period) ?? {} }}
      onClose={onClose}
      onSave={onSave}
    />
  )
}

const OverridePriceModal = ({
  component,
  currency,
  onClose,
  onSave,
}: {
  component: SubscriptionComponent
  currency: string
  onClose: () => void
  onSave: (draft: EditComponentDraft) => void
}) => (
  <FeePriceDialog
    name={component.name}
    fee={component.fee}
    period={component.period}
    currency={currency}
    onClose={onClose}
    onSave={charge =>
      onSave({ subscriptionComponentId: component.id, name: component.name, ...charge })
    }
  />
)

const OverrideAddOnPriceModal = ({
  addOn,
  currency,
  onClose,
  onSave,
}: {
  addOn: SubscriptionAddOn
  currency: string
  onClose: () => void
  onSave: (priceOverride: NonNullable<EditAddOnDraft['priceOverride']>) => void
}) => (
  <FeePriceDialog
    name={addOn.name}
    fee={addOn.fee}
    period={addOn.period}
    currency={currency}
    onClose={onClose}
    onSave={onSave}
  />
)

// --- Add add-on picker ---

const AddAddOnModal = ({
  catalog,
  selectedIds,
  onClose,
  onAdd,
  onRemove,
}: {
  catalog: { id: string; name: string }[]
  selectedIds: string[]
  onClose: () => void
  onAdd: (addOnId: string) => void
  onRemove: (addOnId: string) => void
}) => {
  const [search, setSearch] = useState('')
  const filtered = search
    ? catalog.filter(a => a.name.toLowerCase().includes(search.toLowerCase()))
    : catalog

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[480px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add add-on</DialogTitle>
        </DialogHeader>
        <div className="relative mb-2">
          <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            type="search"
            placeholder="Filter add-ons..."
            value={search}
            onChange={e => setSearch(e.target.value)}
            className="pl-8 h-9"
          />
        </div>
        <div className="space-y-1 max-h-80 overflow-y-auto">
          {catalog.length === 0 && (
            <p className="text-sm text-muted-foreground py-2">No add-ons available.</p>
          )}
          {filtered.map(addOn => {
            const isSelected = selectedIds.includes(addOn.id)
            return (
              <button
                key={addOn.id}
                type="button"
                className={`w-full flex items-center gap-3 px-3 py-2 rounded-md text-left transition-colors ${
                  isSelected
                    ? 'bg-success/10 border border-success/30'
                    : 'hover:bg-muted/50 border border-transparent'
                }`}
                onClick={() => (isSelected ? onRemove(addOn.id) : onAdd(addOn.id))}
              >
                <div
                  className={`shrink-0 w-5 h-5 rounded border flex items-center justify-center ${
                    isSelected
                      ? 'bg-success border-success text-success-foreground'
                      : 'border-border'
                  }`}
                >
                  {isSelected && <Check className="h-3 w-3" />}
                </div>
                <span className="text-sm font-medium">{addOn.name}</span>
              </button>
            )
          })}
        </div>
        <div className="flex justify-end pt-2">
          <Button variant="secondary" onClick={onClose}>
            Done
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
