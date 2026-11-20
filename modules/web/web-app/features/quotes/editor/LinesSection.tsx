import {
  Badge,
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  GenericFormField,
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Skeleton,
  Switch,
} from '@md/ui'
import { Plus, Tag, X } from 'lucide-react'
import { useState } from 'react'
import { useFormContext } from 'react-hook-form'

import { SubscribablePlanVersionSelect } from '@/features/plans/SubscribablePlanVersionSelect'
import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { priceSummaryBadges } from '@/features/plans/pricecomponents/utils'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { couponDiscountLabel } from '@/features/productCatalog/coupons/discountLabel'
import { EditorSection } from '@/features/quotes/editor/EditorSection'
import { LinesTable } from '@/features/quotes/editor/LinesTable'
import { discountError } from '@/features/quotes/editor/quoteV2'
import {
  AddFeeModal,
  OverrideFeeModal,
  pricingTransitions,
} from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'

import type { LinesTableActions } from '@/features/quotes/editor/LinesTable'
import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { EditorLine, QuoteV2State } from '@/features/quotes/editor/quoteV2'
import type { PriceComponentsState } from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { Coupon } from '@/rpc/api/coupons/v1/models_pb'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Dispatch, ReactNode, SetStateAction } from 'react'

type AddOnSelection = { addOnId: string; quantity?: number }
type CouponSelection = { couponId: string }

interface LinesSectionProps {
  customerId?: string
  planVersionId?: string
  currency?: string
  planComponents?: PriceComponent[]
  pricing: PriceComponentsState
  setPricing: Dispatch<SetStateAction<PriceComponentsState>>
  pricingErrors: string[]
  lines: EditorLine[]
  addOns: AddOnSelection[]
  setAddOns: Dispatch<SetStateAction<AddOnSelection[]>>
  availableAddOns: AddOn[]
  coupons: CouponSelection[]
  setCoupons: Dispatch<SetStateAction<CouponSelection[]>>
  availableCoupons: Coupon[]
  exampleAmountByKey: Map<string, string | undefined>
  v2: QuoteV2State
  setV2: Dispatch<SetStateAction<QuoteV2State>>
  lineEditing: boolean
  /** The quote's minimum commitment, when the edition offers it. */
  minimumCommitment?: ReactNode
  /** A quote prices for a customer; a template does not. */
  requireCustomer?: boolean
}

export const LinesSection = ({
  customerId,
  planVersionId,
  currency,
  planComponents,
  pricing,
  setPricing,
  pricingErrors,
  lines,
  addOns,
  setAddOns,
  availableAddOns,
  coupons,
  setCoupons,
  availableCoupons,
  exampleAmountByKey,
  v2,
  setV2,
  lineEditing,
  minimumCommitment,
  requireCustomer = true,
}: LinesSectionProps) => {
  const { control } = useFormContext<QuoteFormValues>()
  const [addingFee, setAddingFee] = useState(false)
  const [overrideComponentId, setOverrideComponentId] = useState<string>()
  const [editExtraIndex, setEditExtraIndex] = useState<number>()

  const actions: LinesTableActions = {
    toggleRemoval: id => setPricing(prev => pricingTransitions.toggleRemoval(prev, id)),
    updateConfiguration: (id, config) =>
      setPricing(prev => pricingTransitions.updateConfiguration(prev, id, config)),
    startOverride: setOverrideComponentId,
    removeOverride: id => setPricing(prev => pricingTransitions.removeOverride(prev, id)),
    editExtra: setEditExtraIndex,
    removeExtra: index => setPricing(prev => pricingTransitions.removeExtra(prev, index)),
    setExampleQuantity: (key, quantity) =>
      setPricing(prev => pricingTransitions.setExampleQuantity(prev, key, quantity)),
    setAddOnQuantity: (id, quantity) =>
      setAddOns(prev => prev.map(a => (a.addOnId === id ? { ...a, quantity } : a))),
    removeAddOn: id => setAddOns(prev => prev.filter(a => a.addOnId !== id)),
  }

  const setDisplay = (patch: Partial<QuoteV2State['displayOptions']>) =>
    setV2(prev => ({ ...prev, displayOptions: { ...prev.displayOptions, ...patch } }))

  const planSelect = (
    <div className="flex flex-col items-end gap-1">
      <GenericFormField
        control={control}
        name="plan_version_id"
        label="Plan"
        layout="vertical"
        containerClassName="w-64 max-w-full"
        render={({ field }) => (
          <SubscribablePlanVersionSelect value={field.value} onChange={field.onChange} />
        )}
      />
      {currency && <span className="text-xs text-muted-foreground">Priced in {currency}</span>}
    </div>
  )

  const content = () => {
    if (!planVersionId) {
      return <p className="text-sm text-muted-foreground">No plan selected</p>
    }
    if (requireCustomer && !customerId) {
      return <p className="text-sm text-muted-foreground">No customer selected</p>
    }
    if (!currency || !planComponents) {
      return (
        <div className="flex flex-col gap-2">
          <Skeleton height={36} />
          <Skeleton height={36} />
          <Skeleton height={36} />
        </div>
      )
    }

    const overridden = overrideComponentId
      ? planComponents.find(c => c.id === overrideComponentId)
      : undefined
    const hasDiscountError = lines.some(l =>
      discountError(v2.lineDetails[l.key]?.discountPercent ?? '')
    )

    return (
      <>
        <LinesTable
          currency={currency}
          planComponents={planComponents}
          pricing={pricing}
          lines={lines}
          addOns={addOns}
          availableAddOns={availableAddOns}
          exampleAmountByKey={exampleAmountByKey}
          v2={v2}
          setV2={setV2}
          lineEditing={lineEditing}
          actions={actions}
        />

        {hasDiscountError && (
          <p className="text-sm text-destructive">A discount is above 0 and at most 100.</p>
        )}
        {pricingErrors.length > 0 && (
          <ul className="flex flex-col gap-0.5 text-sm text-warning">
            {pricingErrors.map(error => (
              <li key={error}>{error}</li>
            ))}
          </ul>
        )}

        <div className="flex flex-wrap items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            hasIcon
            onClick={() => setAddingFee(true)}
          >
            <Plus className="h-4 w-4" />
            Add a fee
          </Button>
          <AddOnPicker
            currency={currency}
            available={availableAddOns.filter(a => !addOns.some(s => s.addOnId === a.id))}
            onPick={id => setAddOns(prev => [...prev, { addOnId: id, quantity: 1 }])}
          />
          <CouponPicker
            available={availableCoupons.filter(c => !coupons.some(s => s.couponId === c.id))}
            onPick={id => setCoupons(prev => [...prev, { couponId: id }])}
          />
          {lineEditing && (
            <div className="ml-auto flex flex-wrap items-center gap-x-5 gap-y-2">
              <div className="flex items-center gap-2">
                <Switch
                  id="quoteShowListPrice"
                  checked={v2.displayOptions.showListPrice}
                  onCheckedChange={checked => setDisplay({ showListPrice: checked })}
                />
                <Label htmlFor="quoteShowListPrice" className="text-sm font-normal">
                  Show list price
                </Label>
              </div>
              <div className="flex items-center gap-2">
                <Switch
                  id="quoteHideZeroLines"
                  checked={v2.displayOptions.hideZeroLines}
                  onCheckedChange={checked => setDisplay({ hideZeroLines: checked })}
                />
                <Label htmlFor="quoteHideZeroLines" className="text-sm font-normal">
                  Hide free lines
                </Label>
              </div>
            </div>
          )}
        </div>

        {coupons.length > 0 && (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs text-muted-foreground">Coupons</span>
            {coupons.map(({ couponId }) => {
              const coupon = availableCoupons.find(c => c.id === couponId)
              const code = coupon?.code ?? 'Coupon'
              return (
                <Badge key={couponId} variant="secondary" className="gap-1.5 pr-1">
                  <Tag className="h-3 w-3" />
                  <span className="font-mono">{code}</span>
                  {coupon && (
                    <span className="font-normal text-muted-foreground">
                      {couponDiscountLabel(coupon.discount)}
                    </span>
                  )}
                  <button
                    type="button"
                    onClick={() => setCoupons(prev => prev.filter(c => c.couponId !== couponId))}
                    className="rounded-sm p-0.5 hover:bg-muted"
                    aria-label={`Remove coupon ${code}`}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </Badge>
              )
            })}
          </div>
        )}

        {minimumCommitment && (
          <div className="border-t border-border pt-4">{minimumCommitment}</div>
        )}

        {addingFee && (
          <AddFeeModal
            onClose={() => setAddingFee(false)}
            onAdd={component => setPricing(prev => pricingTransitions.addExtra(prev, component))}
            currency={currency}
          />
        )}
        {overrideComponentId && (
          <OverrideFeeModal
            componentId={overrideComponentId}
            originalComponent={overridden}
            onClose={() => setOverrideComponentId(undefined)}
            onSave={override => {
              setPricing(prev =>
                pricingTransitions.addOverride(prev, overrideComponentId, override)
              )
              setOverrideComponentId(undefined)
            }}
            currency={currency}
          />
        )}
        {editExtraIndex !== undefined && pricing.components.extra[editExtraIndex] && (
          <AddFeeModal
            onClose={() => setEditExtraIndex(undefined)}
            onAdd={component => {
              setPricing(prev => pricingTransitions.updateExtra(prev, editExtraIndex, component))
              setEditExtraIndex(undefined)
            }}
            currency={currency}
            initialValues={pricing.components.extra[editExtraIndex]}
            isEditing
          />
        )}
      </>
    )
  }

  return (
    <EditorSection id="quote-lines" title="Plan & lines" aside={planSelect}>
      {content()}
    </EditorSection>
  )
}

const AddOnPicker = ({
  currency,
  available,
  onPick,
}: {
  currency: string
  available: AddOn[]
  onPick: (addOnId: string) => void
}) => {
  const [open, setOpen] = useState(false)
  const creditUnits = useCreditUnits()

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button type="button" variant="ghost" size="sm" hasIcon disabled={available.length === 0}>
          <Plus className="h-4 w-4" />
          Add-on
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-80 p-0" align="start">
        <Command>
          <CommandInput placeholder="Search add-ons" />
          <CommandList>
            <CommandEmpty>No matching add-on</CommandEmpty>
            <CommandGroup>
              {available.map(addOn => (
                <CommandItem
                  key={addOn.id}
                  value={`${addOn.name} ${addOn.id}`}
                  onSelect={() => {
                    onPick(addOn.id)
                    setOpen(false)
                  }}
                  className="flex items-center justify-between gap-3"
                >
                  <span className="truncate">{addOn.name}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {priceSummaryBadges(
                      feeTypeEnumToComponentFeeType(addOn.feeType),
                      addOn.price,
                      currency,
                      creditUnits
                    ).join(' / ')}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

const CouponPicker = ({
  available,
  onPick,
}: {
  available: Coupon[]
  onPick: (couponId: string) => void
}) => {
  const [open, setOpen] = useState(false)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button type="button" variant="ghost" size="sm" hasIcon disabled={available.length === 0}>
          <Plus className="h-4 w-4" />
          Coupon
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-72 p-0" align="start">
        <Command>
          <CommandInput placeholder="Search coupons" />
          <CommandList>
            <CommandEmpty>No matching coupon</CommandEmpty>
            <CommandGroup>
              {available.map(coupon => (
                <CommandItem
                  key={coupon.id}
                  value={`${coupon.code} ${coupon.id}`}
                  onSelect={() => {
                    onPick(coupon.id)
                    setOpen(false)
                  }}
                  className="flex items-center justify-between gap-3"
                >
                  <span className="truncate font-mono">{coupon.code}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {couponDiscountLabel(coupon.discount)}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
