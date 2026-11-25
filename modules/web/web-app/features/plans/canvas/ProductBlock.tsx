import { cn } from '@md/ui'
import { ReactNode } from 'react'

import { useComponentProduct } from '@/features/plans/pricecomponents/componentRefs'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'

import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

const SHOWN_FEATURES = 5

/** The charges of one product, grouped: a component without a product is its own group. */
export const groupByProduct = (components: PriceComponent[]) => {
  const groups = new Map<string, PriceComponent[]>()
  for (const component of components) {
    const key = component.productId ?? `component:${component.id}`
    groups.set(key, [...(groups.get(key) ?? []), component])
  }
  return [...groups.entries()].map(([key, charges]) => ({ key, charges }))
}

/** A product of the plan: what it is and the features it brings, then its charges. */
export const ProductBlock = ({
  productId,
  fallbackName,
  features,
  actions,
  highlighted = false,
  children,
}: {
  productId?: string
  fallbackName: string
  features: string[]
  /** "All prices" and the product's menu: shown at the right of its header. */
  actions?: ReactNode
  /** Just added, or sent to: lit for a moment, the light fading as it goes off. */
  highlighted?: boolean
  children: ReactNode
}) => {
  const product = useComponentProduct(productId)
  const more = features.length - SHOWN_FEATURES
  const featureLine = [
    ...features.slice(0, SHOWN_FEATURES),
    ...(more > 0 ? [`${more} more`] : []),
  ].join(' · ')
  return (
    <section
      className={cn(
        'overflow-hidden rounded-lg border bg-card transition-[border-color,box-shadow] duration-700',
        highlighted ? 'border-brand ring-2 ring-brand/25' : 'border-border'
      )}
    >
      <header className="flex items-center gap-3 border-b border-border px-4 py-3">
        <ProductIcon icon={product?.icon} color={product?.color} />
        <div className="min-w-0 flex-1">
          <h4 className="truncate text-sm font-semibold text-foreground">
            {product?.name ?? fallbackName}
          </h4>
          {(product?.description || featureLine) && (
            <p className="line-clamp-2 text-xs text-muted-foreground">
              {product?.description || featureLine}
            </p>
          )}
          {product?.description && featureLine && (
            <p className="truncate text-xs text-muted-foreground/80" aria-label="Features">
              {featureLine}
            </p>
          )}
        </div>
        {actions && <div className="flex shrink-0 items-center gap-1.5">{actions}</div>}
      </header>
      <div className="divide-y divide-border">{children}</div>
    </section>
  )
}
