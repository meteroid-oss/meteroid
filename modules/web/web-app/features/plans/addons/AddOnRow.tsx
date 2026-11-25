import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@md/ui'
import { MoreHorizontalIcon, Trash2Icon } from 'lucide-react'
import { Link } from 'react-router'

import { Gate } from '@/components/Gate'
import { PriceChip } from '@/features/plans/canvas/ChargeRow'
import { addOnLine, addOnPrices } from '@/features/productCatalog/addons/addonWords'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'
import { useBasePath } from '@/hooks/useBasePath'

import type { ChargeContext } from '@/features/plans/canvas/billingWords'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'

type Props = {
  addOn: AddOn
  /** The currency the plan editor shows: the add-on's price in it. */
  currency: string
  /** Where the plan is sold and the add-on has no price: "EUR: no price", "USD Annual: no price". */
  gaps: string[]
  isDraft: boolean
  onRemove: () => void
}

/**
 * An add-on offered with the plan, as the plan's charges read: its name, what it charges and how
 * it is sold, its price chip. Its price is the catalog's, the same on every plan: edited there.
 */
export const AddOnRow = ({ addOn, currency, gaps, isDraft, onRemove }: Props) => {
  const basePath = useBasePath()
  const prices = addOnPrices(addOn)
  const ctx: ChargeContext = { product: { feeStructure: addOn.feeStructure } }

  return (
    <div className="group px-4 py-3">
      <div className="flex min-h-7 items-center gap-2">
        <ProductIcon icon={addOn.productIcon} color={addOn.productColor} size="sm" />
        <div className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2">
          <span className="truncate text-sm font-semibold text-foreground">{addOn.name}</span>
          <span className="text-xs text-muted-foreground">{addOnLine(addOn)}</span>
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          <Gate perm="catalog:manage">
            <Button
              asChild
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
            >
              <Link to={`${basePath}/addons/${addOn.id}`}>Edit in catalog</Link>
            </Button>
          </Gate>
          {isDraft && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7 text-muted-foreground"
                  aria-label={`${addOn.name} actions`}
                >
                  <MoreHorizontalIcon size={14} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={onRemove} className="text-destructive">
                  <Trash2Icon size={12} className="mr-2" /> Stop offering it
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-2 pl-7">
        <PriceChip
          prices={prices}
          currency={currency}
          missing={`No price in ${currency}`}
          ctx={ctx}
          isDraft={false}
          onEdit={() => undefined}
        />
        {gaps.slice(0, 2).map(gap => (
          <span
            key={gap}
            className="inline-flex items-baseline rounded-md border border-dashed border-warning/60 px-2.5 py-0.5 text-xs text-warning"
          >
            {gap}
          </span>
        ))}
        {gaps.length > 2 && (
          <span className="text-xs text-warning">and {gaps.length - 2} more</span>
        )}
      </div>
    </div>
  )
}
