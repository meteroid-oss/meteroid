import { Badge, Button, Checkbox, Input, cn } from '@md/ui'
import { Gift, Minus, Plus, Search, Tag } from 'lucide-react'
import { useState } from 'react'

import { billedWords, chipParts, countsWords, priceWords } from '@/features/plans/canvas/billingWords'
import { couponDiscountLabel } from '@/features/productCatalog/coupons/discountLabel'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'

import { addOnPriceIn } from './addOnPrice'

import type { ChargeContext } from '@/features/plans/canvas/billingWords'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { Coupon } from '@/rpc/api/coupons/v1/models_pb'

interface AddOnCouponSelectorProps {
  selectedAddOns: { addOnId: string; quantity?: number }[]
  onAddOnAdd: (addOnId: string) => void
  onAddOnRemove: (addOnId: string) => void
  onAddOnQuantityChange?: (addOnId: string, quantity: number) => void
  availableAddOns: AddOn[]
  selectedCoupons: { couponId: string }[]
  onCouponAdd: (couponId: string) => void
  onCouponRemove: (couponId: string) => void
  availableCoupons: Coupon[]
  isCouponAvailable?: (coupon: Coupon) => boolean
  currency?: string
}

export const AddOnCouponSelector = ({
  selectedAddOns,
  onAddOnAdd,
  onAddOnRemove,
  onAddOnQuantityChange,
  availableAddOns,
  selectedCoupons,
  onCouponAdd,
  onCouponRemove,
  availableCoupons,
  isCouponAvailable,
  currency,
}: AddOnCouponSelectorProps) => {
  const [addOnSearch, setAddOnSearch] = useState('')
  const [couponSearch, setCouponSearch] = useState('')

  const filteredAddOns = addOnSearch
    ? availableAddOns.filter(a => a.name.toLowerCase().includes(addOnSearch.toLowerCase()))
    : availableAddOns

  const filteredCoupons = couponSearch
    ? availableCoupons.filter(c => c.code.toLowerCase().includes(couponSearch.toLowerCase()))
    : availableCoupons

  return (
    <div className="space-y-4">
      {/* Add-ons Section */}
      <div>
        <h3 className="text-sm font-medium mb-3 flex items-center gap-2">
          <Plus className="h-4 w-4 text-success" />
          Add-ons
          {selectedAddOns.length > 0 && (
            <Badge variant="outline" size="sm">
              {selectedAddOns.length} selected
            </Badge>
          )}
        </h3>

        {availableAddOns.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No add-ons available for this plan.
          </p>
        ) : (
          <>
            {availableAddOns.length > 5 && (
              <div className="relative mb-2">
                <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
                <Input
                  type="search"
                  placeholder="Filter add-ons..."
                  value={addOnSearch}
                  onChange={e => setAddOnSearch(e.target.value)}
                  className="pl-8 h-9"
                />
              </div>
            )}
            <div className="space-y-1 max-h-64 overflow-y-auto">
              {filteredAddOns.map(addOn => {
                const selectedEntry = selectedAddOns.find(a => a.addOnId === addOn.id)
                return (
                  <AddOnOption
                    key={addOn.id}
                    addOn={addOn}
                    currency={currency}
                    quantity={selectedEntry ? (selectedEntry.quantity ?? 1) : undefined}
                    onAdd={() => onAddOnAdd(addOn.id)}
                    onRemove={() => onAddOnRemove(addOn.id)}
                    onQuantityChange={
                      onAddOnQuantityChange && (qty => onAddOnQuantityChange(addOn.id, qty))
                    }
                  />
                )
              })}
              {addOnSearch && filteredAddOns.length === 0 && (
                <p className="text-sm text-muted-foreground py-2 text-center">No matching add-ons</p>
              )}
            </div>
          </>
        )}
      </div>

      {/* Coupons Section */}
      <div className="border-t pt-4">
        <h3 className="text-sm font-medium mb-3 flex items-center gap-2">
          <Tag className="h-4 w-4 text-brand" />
          Discount Coupons
          {selectedCoupons.length > 0 && (
            <Badge variant="outline" size="sm">
              {selectedCoupons.length} applied
            </Badge>
          )}
        </h3>

        {availableCoupons.length === 0 ? (
          <p className="text-sm text-muted-foreground">No active coupons available.</p>
        ) : (
          <>
            {availableCoupons.length > 5 && (
              <div className="relative mb-2">
                <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
                <Input
                  type="search"
                  placeholder="Filter coupons..."
                  value={couponSearch}
                  onChange={e => setCouponSearch(e.target.value)}
                  className="pl-8 h-9"
                />
              </div>
            )}
            <div className="space-y-1 max-h-48 overflow-y-auto">
              {filteredCoupons.map(coupon => {
                const isSelected = selectedCoupons.some(c => c.couponId === coupon.id)
                const isAvailable = isCouponAvailable ? isCouponAvailable(coupon) : true
                const discountLabel = couponDiscountLabel(coupon.discount)

                const disabled = !isAvailable && !isSelected

                return (
                  <label
                    key={coupon.id}
                    className={`w-full flex items-center gap-3 px-3 py-2 rounded-md transition-colors ${
                      isSelected
                        ? 'bg-brand/10 border border-brand/30 cursor-pointer'
                        : disabled
                          ? 'opacity-50 cursor-not-allowed border border-transparent'
                          : 'hover:bg-muted/50 border border-transparent cursor-pointer'
                    }`}
                  >
                    <Checkbox
                      checked={isSelected}
                      disabled={disabled}
                      onCheckedChange={checked =>
                        checked ? onCouponAdd(coupon.id) : onCouponRemove(coupon.id)
                      }
                      className="shrink-0 border-border data-[state=checked]:border-brand"
                    />
                    <div className="flex-1 min-w-0 flex items-center gap-2">
                      <Gift className="h-3 w-3 text-muted-foreground shrink-0" />
                      <span className="text-sm font-medium font-mono">{coupon.code}</span>
                    </div>
                    <Badge variant="secondary" size="sm" className="shrink-0">
                      {discountLabel}
                    </Badge>
                    {disabled && (
                      <span className="text-xs text-muted-foreground shrink-0">
                        Not for this plan
                      </span>
                    )}
                  </label>
                )
              })}
              {couponSearch && filteredCoupons.length === 0 && (
                <p className="text-sm text-muted-foreground py-2 text-center">
                  No matching coupons
                </p>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  )
}

/**
 * An add-on as the plan editor reads it: its name, what it charges and how it bills, its price
 * in the subscription's currency; once picked, how many when it can be taken several times.
 */
const AddOnOption = ({
  addOn,
  currency,
  quantity,
  onAdd,
  onRemove,
  onQuantityChange,
}: {
  addOn: AddOn
  currency?: string
  /** Picked: how many. */
  quantity?: number
  onAdd: () => void
  onRemove: () => void
  onQuantityChange?: (quantity: number) => void
}) => {
  const isSelected = quantity !== undefined
  // Sold at its price in the subscription's currency only.
  const price = addOnPriceIn(addOn, currency)
  const unpriced = !price && Boolean(currency)
  const ctx: ChargeContext = { product: { feeStructure: addOn.feeStructure } }
  const what = price ? [countsWords(price, ctx), billedWords(price, ctx)].join(' · ') : undefined
  const amount = price && chipParts(priceWords(price, ctx))
  // Several of it when it allows more than one (none set: any number).
  const maxInstances = addOn.maxInstancesPerSubscription ?? null
  const supportsMultiple = maxInstances === null || maxInstances > 1
  const showQtyControls = isSelected && supportsMultiple && Boolean(onQuantityChange)
  const currentQty = quantity ?? 1

  return (
    <label
      className={cn(
        'flex items-center gap-3 rounded-md border px-3 py-2 transition-colors',
        isSelected
          ? 'border-success/30 bg-success/10'
          : unpriced
            ? 'cursor-not-allowed border-transparent opacity-60'
            : 'cursor-pointer border-transparent hover:bg-muted/50'
      )}
    >
      <Checkbox
        checked={isSelected}
        disabled={unpriced && !isSelected}
        onCheckedChange={checked => (checked ? onAdd() : onRemove())}
        className="shrink-0 data-[state=checked]:bg-success data-[state=checked]:border-success data-[state=checked]:text-success-foreground"
      />
      <ProductIcon icon={addOn.productIcon} color={addOn.productColor} size="sm" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{addOn.name}</span>
        <span className="block truncate text-xs text-muted-foreground">
          {what ?? `No price in ${currency}`}
        </span>
      </span>
      {showQtyControls && (
        <span className="flex shrink-0 items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0"
            aria-label="One less"
            onClick={e => {
              e.preventDefault()
              if (currentQty > 1) onQuantityChange!(currentQty - 1)
            }}
            disabled={currentQty <= 1}
          >
            <Minus className="h-3 w-3" />
          </Button>
          <span className="w-5 text-center text-sm tabular-nums">{currentQty}</span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0"
            aria-label="One more"
            onClick={e => {
              e.preventDefault()
              if (maxInstances === null || currentQty < maxInstances) {
                onQuantityChange!(currentQty + 1)
              }
            }}
            disabled={maxInstances !== null && currentQty >= maxInstances}
          >
            <Plus className="h-3 w-3" />
          </Button>
        </span>
      )}
      {amount && (
        <span className="shrink-0 text-right">
          {isSelected && currentQty > 1 && (
            <span className="text-xs text-muted-foreground">{currentQty} × </span>
          )}
          <span className="text-sm font-semibold tabular-nums">{amount.strong}</span>
          {amount.quiet && (
            <span className="ml-1 text-xs text-muted-foreground">{amount.quiet}</span>
          )}
        </span>
      )}
    </label>
  )
}
