import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
} from '@md/ui'
import { useAtom } from 'jotai'
import { Check, Minus, Pencil, Plus, Search, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useWizard } from 'react-use-wizard'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { FeeTypePicker } from '@/features/plans/pricecomponents/FeeTypePicker'
import { ProductBrowser } from '@/features/plans/pricecomponents/ProductBrowser'
import { ProductPricingForm } from '@/features/plans/pricecomponents/ProductPricingForm'
import { priceSummaryBadges } from '@/features/plans/pricecomponents/utils'
import { formDataToPrice } from '@/features/pricing'
import {
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
import { FeeStructure_BillingType } from '@/rpc/api/prices/v1/models_pb'
import {
  SubscriptionAddOn,
  SubscriptionComponent,
  SubscriptionFee,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { StructuralInfo } from '@/features/plans/pricecomponents/ProductPricingForm'
import type { ComponentFeeType } from '@/features/pricing'

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
    default:
      return 'rate'
  }
}

const usageModelProtoToForm: Record<string, string> = {
  perUnit: 'per_unit',
  tiered: 'tiered',
  volume: 'volume',
  package: 'package',
  matrix: 'matrix',
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

/**
 * Structural (non-price) context extracted from a live fee, so the pricing form
 * can display/resolve the metric, usage model, slot unit, etc. when overriding.
 */
const structuralFromFee = (fee?: SubscriptionFee): StructuralInfo => {
  switch (fee?.fee.case) {
    case 'capacity':
      return { metricId: fee.fee.value.metricId }
    case 'usage':
      return {
        metricId: fee.fee.value.metricId,
        usageModel: usageModelProtoToForm[fee.fee.value.model.case ?? ''] ?? 'per_unit',
      }
    case 'slot':
      return { slotUnitName: fee.fee.value.unit }
    case 'recurring':
      return {
        billingType:
          fee.fee.value.billingType === FeeStructure_BillingType.ADVANCE ? 'ADVANCE' : 'ARREAR',
      }
    default:
      return {}
  }
}

/**
 * Seed an override price form from a live fee. Covers scalar types fully
 * (rate / one-time / recurring / capacity / slot) and usage per-unit. For
 * tiered / volume / package / matrix usage, the structural context (metric +
 * model) is seeded but the rate rows are left for re-entry, since reconstructing
 * them from the runtime fee is lossy.
 */
const subscriptionFeeToFormData = (
  fee: SubscriptionFee | undefined,
  period: SubscriptionFeeBillingPeriod
): Record<string, unknown> | undefined => {
  const f = fee?.fee
  const term = periodToCadence(period)
  switch (f?.case) {
    case 'rate':
      return { term, rate: f.value.rate }
    case 'oneTime':
      return { unitPrice: f.value.rate, quantity: f.value.quantity }
    case 'recurring':
      return {
        term,
        billingType:
          f.value.billingType === FeeStructure_BillingType.ADVANCE ? 'ADVANCE' : 'ARREAR',
        unitPrice: f.value.rate,
        quantity: f.value.quantity,
      }
    case 'capacity':
      return {
        term,
        rate: f.value.rate,
        included: Number(f.value.included),
        overageRate: f.value.overageRate,
      }
    case 'slot':
      return {
        term,
        slotUnitName: f.value.unit,
        unitRate: f.value.unitRate,
        minSlots: f.value.minSlots,
        maxSlots: f.value.maxSlots,
        minimumCount: f.value.minSlots ?? 1,
        // Each policy enum currently has a single variant.
        upgradePolicy: 'PRORATED',
        downgradePolicy: 'REMOVE_AT_END_OF_PERIOD',
      }
    case 'usage': {
      const model = f.value.model
      const usageModel = usageModelProtoToForm[model.case ?? ''] ?? 'per_unit'
      const base = { metricId: f.value.metricId, usageModel, term }
      if (model.case === 'perUnit') {
        return { ...base, unitPrice: model.value }
      }
      // tiered / volume / package / matrix: structure only; rates re-entered.
      return base
    }
    default:
      return undefined
  }
}

/**
 * Render a draft (form-data) price the same way live component fees are shown,
 * by converting form data -> Price -> SubscriptionFee. Falls back to the fee
 * type name if the form data can't be converted yet.
 */
const formatDraftFee = (
  feeType: ComponentFeeType,
  formData: Record<string, unknown>,
  currency: string
): string => {
  try {
    return formatSubscriptionFeeCompact(
      priceToSubscriptionFee(formDataToPrice(feeType, formData, currency)),
      currency
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

  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId: state.subscriptionId },
    { enabled: Boolean(state.subscriptionId) }
  )

  const components = subscriptionQuery.data?.priceComponents ?? []
  const addOns = subscriptionQuery.data?.addOns ?? []

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
                        {edited && (
                          <Badge variant="outline" size="sm" className="ml-2 text-brand border-brand/30">
                            Price overridden
                          </Badge>
                        )}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        {edited ? (
                          <>
                            <span className="line-through opacity-60">
                              {formatSubscriptionFeeCompact(component.fee, currency)}
                            </span>{' '}
                            <span className="text-brand">
                              {formatDraftFee(edited.feeType, edited.formData, currency)}
                            </span>
                          </>
                        ) : (
                          formatSubscriptionFeeCompact(component.fee, currency)
                        )}
                        <span className="opacity-70">
                          {' · '}
                          {formatSubscriptionFeeBillingPeriod(component.period)}
                        </span>
                      </div>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {edited && !isRemoved && (
                        <Button variant="ghost" size="sm" onClick={() => clearOverride(component.id)}>
                          Reset
                        </Button>
                      )}
                      {!isRemoved && (
                        <Button
                          variant="secondary"
                          size="sm"
                          onClick={() => setOverrideComponent(component)}
                        >
                          <Pencil className="h-4 w-4 mr-1" /> Override price
                        </Button>
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
                      <Badge variant="outline" size="sm" className="ml-2 text-brand border-brand/30">
                        Added
                      </Badge>
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {formatDraftFee(extra.feeType, extra.formData, currency)}
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
                          <Badge variant="outline" size="sm" className="ml-2 text-brand border-brand/30">
                            Qty {addOn.quantity} → {edited.quantity}
                          </Badge>
                        )}
                        {edited?.priceOverride && (
                          <Badge variant="outline" size="sm" className="ml-2 text-brand border-brand/30">
                            Price overridden
                          </Badge>
                        )}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        {edited?.priceOverride ? (
                          <>
                            <span className="line-through opacity-60">
                              {formatSubscriptionFeeCompact(addOn.fee, currency)}
                            </span>{' '}
                            <span className="text-brand">
                              {formatDraftFee(
                                edited.priceOverride.feeType,
                                edited.priceOverride.formData,
                                currency
                              )}
                            </span>
                          </>
                        ) : (
                          formatSubscriptionFeeCompact(addOn.fee, currency)
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
                        <Button
                          variant="secondary"
                          size="sm"
                          onClick={() => setOverrideAddOn(addOn)}
                        >
                          <Pencil className="h-4 w-4 mr-1" /> Override price
                        </Button>
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
                      feeTypeEnumToComponentFeeType(cat.feeType),
                      cat.price,
                      currency
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
                        <Badge variant="outline" size="sm" className="ml-2 text-brand border-brand/30">
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
                      <Button variant="ghost" size="sm" onClick={() => removeNewAddOn(added.addOnId)}>
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

      {/* Add ad-hoc component modal */}
      {showAddComponent && (
        <AddComponentModal
          currency={currency}
          onClose={() => setShowAddComponent(false)}
          onAdd={addExtraComponent}
        />
      )}

      {/* Edit an added ad-hoc component */}
      {editExtraIndex !== null && state.componentChanges.added[editExtraIndex] && (
        <EditExtraComponentModal
          draft={state.componentChanges.added[editExtraIndex]}
          currency={currency}
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

// --- Override price modal ---

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
}) => {
  const feeType = feeTypeFromSubscriptionFee(component.fee)
  const initialFormData = subscriptionFeeToFormData(component.fee, component.period)
  const structuralInfo = structuralFromFee(component.fee)

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[500px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Override price: {component.name}</DialogTitle>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          Enter the new price for this component. The current fee is&nbsp;
          {formatSubscriptionFeeCompact(component.fee, currency)}.
        </p>
        <ProductPricingForm
          feeType={feeType}
          currency={currency}
          editableStructure={!component.productId}
          isOverride
          initialFormData={initialFormData}
          structuralInfo={structuralInfo}
          onSubmit={formData =>
            onSave({
              subscriptionComponentId: component.id,
              name: component.name,
              feeType,
              formData,
            })
          }
          submitLabel="Save override"
        />
      </DialogContent>
    </Dialog>
  )
}

// --- Override an existing add-on's price ---

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
}) => {
  const feeType = feeTypeFromSubscriptionFee(addOn.fee)
  const initialFormData = subscriptionFeeToFormData(addOn.fee, addOn.period)
  const structuralInfo = structuralFromFee(addOn.fee)

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[500px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Override price: {addOn.name}</DialogTitle>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          Enter the new price for this add-on. The current fee is&nbsp;
          {formatSubscriptionFeeCompact(addOn.fee, currency)}.
        </p>
        <ProductPricingForm
          feeType={feeType}
          currency={currency}
          editableStructure={false}
          isOverride
          initialFormData={initialFormData}
          structuralInfo={structuralInfo}
          onSubmit={formData => onSave({ feeType, formData })}
          submitLabel="Save override"
        />
      </DialogContent>
    </Dialog>
  )
}

// --- Edit an added ad-hoc component (name + price) ---

const EditExtraComponentModal = ({
  draft,
  currency,
  onClose,
  onSave,
}: {
  draft: ExtraComponentDraft
  currency: string
  onClose: () => void
  onSave: (draft: ExtraComponentDraft) => void
}) => {
  const [name, setName] = useState(draft.name)

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[500px] max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Edit component</DialogTitle>
        </DialogHeader>
        <div>
          <Label className="text-sm">Component name</Label>
          <Input
            className="mt-1"
            value={name}
            onChange={e => setName(e.target.value)}
            autoFocus
          />
        </div>
        <ProductPricingForm
          feeType={draft.feeType}
          currency={currency}
          editableStructure={!draft.productId}
          initialFormData={draft.formData}
          structuralInfo={{
            metricId: draft.formData.metricId as string | undefined,
            usageModel: draft.formData.usageModel as string | undefined,
            slotUnitName: draft.formData.slotUnitName as string | undefined,
            billingType: draft.formData.billingType as string | undefined,
          }}
          onSubmit={formData =>
            onSave({ ...draft, name, formData: { ...draft.formData, ...formData } })
          }
          submitLabel="Save changes"
        />
      </DialogContent>
    </Dialog>
  )
}

// --- Add ad-hoc component modal (product library + custom fee) ---

const AddComponentModal = ({
  currency,
  onClose,
  onAdd,
}: {
  currency: string
  onClose: () => void
  onAdd: (component: ExtraComponentDraft) => void
}) => {
  const [step, setStep] = useState<'choose' | 'identity' | 'feeType' | 'pricing'>('choose')
  const [name, setName] = useState('')
  const [feeType, setFeeType] = useState<ComponentFeeType | null>(null)

  const handleProductAdd = ({
    productId,
    componentName,
    formData,
    feeType: ft,
  }: {
    productId: string
    componentName: string
    formData: Record<string, unknown>
    feeType: ComponentFeeType
  }) => {
    onAdd({ name: componentName, feeType: ft, formData, productId })
  }

  const handleCustomSubmit = (formData: Record<string, unknown>) => {
    if (!feeType) return
    onAdd({ name, feeType, formData, productId: undefined })
  }

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[600px] max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add component</DialogTitle>
        </DialogHeader>

        {step === 'choose' && (
          <div className="space-y-4">
            <ProductBrowser currency={currency} onAdd={handleProductAdd} submitLabel="Add component" />
            <div className="border-t pt-3">
              <Button variant="secondary" onClick={() => setStep('identity')}>
                Or create a custom fee
              </Button>
            </div>
          </div>
        )}

        {step === 'identity' && (
          <div className="space-y-4">
            <div>
              <Label className="text-sm">Component name</Label>
              <Input
                className="mt-1"
                placeholder="e.g. Setup fee"
                value={name}
                onChange={e => setName(e.target.value)}
                autoFocus
              />
            </div>
            <div className="flex justify-between">
              <Button variant="ghost" onClick={() => setStep('choose')}>
                Back
              </Button>
              <Button disabled={name.length === 0} onClick={() => setStep('feeType')}>
                Next
              </Button>
            </div>
          </div>
        )}

        {step === 'feeType' && (
          <div className="space-y-4">
            <button
              type="button"
              onClick={() => setStep('identity')}
              className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
            >
              Back
            </button>
            <div className="text-sm font-medium">{name}</div>
            <FeeTypePicker
              onSelect={ft => {
                setFeeType(ft)
                setStep('pricing')
              }}
            />
          </div>
        )}

        {step === 'pricing' && feeType && (
          <div className="space-y-4">
            <button
              type="button"
              onClick={() => setStep('feeType')}
              className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
            >
              Back
            </button>
            <div className="text-sm font-medium">{name}</div>
            <ProductPricingForm
              feeType={feeType}
              currency={currency}
              editableStructure
              onSubmit={handleCustomSubmit}
              submitLabel="Add component"
            />
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

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
                  className={`flex-shrink-0 w-5 h-5 rounded border flex items-center justify-center ${
                    isSelected ? 'bg-success border-success text-success-foreground' : 'border-border'
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
