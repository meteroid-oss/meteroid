import { Badge, Input } from '@md/ui'
import { Check, ChevronDownIcon, ChevronRightIcon, Gift, Plus, Search, Tag } from 'lucide-react'
import { useState } from 'react'

import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { PricingDetailsView } from '@/features/plans/pricecomponents/components/PricingDetailsView'
import { feeTypeToHuman, priceSummaryBadges } from '@/features/plans/pricecomponents/utils'

import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { Coupon } from '@/rpc/api/coupons/v1/models_pb'

interface AddOnCouponSelectorProps {
  selectedAddOns: { addOnId: string }[]
  onAddOnAdd: (addOnId: string) => void
  onAddOnRemove: (addOnId: string) => void
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
  const [expandedAddOnId, setExpandedAddOnId] = useState<string | null>(null)

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
                const isSelected = selectedAddOns.some(a => a.addOnId === addOn.id)
                const isExpanded = expandedAddOnId === addOn.id
                const feeType = feeTypeEnumToComponentFeeType(addOn.feeType)
                const feeLabel = feeTypeToHuman(feeType)
                const priceBadge = priceSummaryBadges(feeType, addOn.price, currency).join(' / ')
                const addOnCurrency = currency ?? addOn.price?.currency ?? 'USD'

                return (
                  <div
                    key={addOn.id}
                    className={`rounded-md transition-colors ${
                      isSelected
                        ? 'bg-success/10 border border-success/30'
                        : 'border border-transparent hover:bg-muted/50'
                    }`}
                  >
                    <div className="flex items-center gap-3 px-3 py-2">
                      <button
                        type="button"
                        className="flex-shrink-0 text-muted-foreground hover:text-foreground"
                        onClick={() => setExpandedAddOnId(isExpanded ? null : addOn.id)}
                      >
                        {isExpanded ? (
                          <ChevronDownIcon className="w-4 h-4" />
                        ) : (
                          <ChevronRightIcon className="w-4 h-4" />
                        )}
                      </button>
                      <button
                        type="button"
                        className="flex items-center gap-3 flex-1 min-w-0 text-left"
                        onClick={() => (isSelected ? onAddOnRemove(addOn.id) : onAddOnAdd(addOn.id))}
                      >
                        <div
                          className={`flex-shrink-0 w-5 h-5 rounded border flex items-center justify-center ${
                            isSelected
                              ? 'bg-success border-success text-success-foreground'
                              : 'border-border'
                          }`}
                        >
                          {isSelected && <Check className="h-3 w-3" />}
                        </div>
                        <div className="flex-1 min-w-0">
                          <span className="text-sm font-medium">{addOn.name}</span>
                        </div>
                        <Badge variant="outline" size="sm" className="flex-shrink-0">
                          {feeLabel}
                        </Badge>
                        <span className="text-xs text-muted-foreground flex-shrink-0">
                          {priceBadge}
                        </span>
                      </button>
                    </div>
                    {isExpanded && addOn.price && (
                      <div className="px-3 pb-3 pt-0 border-t border-border mx-3 mt-0">
                        <PricingDetailsView prices={[addOn.price]} currency={addOnCurrency} />
                      </div>
                    )}
                  </div>
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
                const discountLabel =
                  coupon.discount?.discountType?.case === 'percentage'
                    ? `${coupon.discount.discountType.value.percentage}% off`
                    : coupon.discount?.discountType?.case === 'fixed'
                      ? `${coupon.discount.discountType.value.amount} ${coupon.discount.discountType.value.currency} off`
                      : 'Discount'

                return (
                  <button
                    key={coupon.id}
                    type="button"
                    disabled={!isAvailable && !isSelected}
                    className={`w-full flex items-center gap-3 px-3 py-2 rounded-md text-left transition-colors ${
                      isSelected
                        ? 'bg-brand/10 border border-brand/30'
                        : !isAvailable
                          ? 'opacity-50 cursor-not-allowed border border-transparent'
                          : 'hover:bg-muted/50 border border-transparent'
                    }`}
                    onClick={() =>
                      isSelected ? onCouponRemove(coupon.id) : onCouponAdd(coupon.id)
                    }
                  >
                    <div
                      className={`flex-shrink-0 w-5 h-5 rounded border flex items-center justify-center ${
                        isSelected
                          ? 'bg-brand border-brand text-brand-foreground'
                          : 'border-border'
                      }`}
                    >
                      {isSelected && <Check className="h-3 w-3" />}
                    </div>
                    <div className="flex-1 min-w-0 flex items-center gap-2">
                      <Gift className="h-3 w-3 text-muted-foreground flex-shrink-0" />
                      <span className="text-sm font-medium font-mono">{coupon.code}</span>
                    </div>
                    <Badge variant="secondary" size="sm" className="flex-shrink-0">
                      {discountLabel}
                    </Badge>
                    {!isAvailable && !isSelected && (
                      <span className="text-xs text-muted-foreground flex-shrink-0">
                        Not for this plan
                      </span>
                    )}
                  </button>
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
