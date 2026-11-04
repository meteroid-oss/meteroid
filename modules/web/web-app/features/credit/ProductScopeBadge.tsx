import { Badge } from '@md/ui'

type NamedProduct = { id: string; name: string }

const SHOWN_NAMES = 2

/** "Restricted to A, B +N", or undefined for credit that applies to every product. */
export const productScopeLabel = (products: NamedProduct[]): string | undefined => {
  if (products.length === 0) return undefined
  const shown = products
    .slice(0, SHOWN_NAMES)
    .map(product => product.name)
    .join(', ')
  const more = products.length - SHOWN_NAMES
  return `Restricted to ${shown}${more > 0 ? ` +${more}` : ''}`
}

export const ProductScopeBadge = ({ products }: { products: NamedProduct[] }) => {
  const label = productScopeLabel(products)
  if (!label) return null
  return (
    <Badge size="sm" variant="outline" title={products.map(product => product.name).join(', ')}>
      {label}
    </Badge>
  )
}
