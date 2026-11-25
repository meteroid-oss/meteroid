import { Badge, Input } from '@md/ui'
import { Loader2Icon, SearchIcon } from 'lucide-react'
import { createElement, useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { feeTypeIcon, priceSummaryBadges } from '@/features/plans/pricecomponents/utils'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { protoFeeTypeToComponentFeeType } from '@/features/pricing/formData'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listProductsWithPrices } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import type { ComponentFeeType } from '@/features/pricing/conversions'
import type { ProductWithPrice } from '@/rpc/api/products/v1/models_pb'

function feeTypeColor(type: ComponentFeeType): { bg: string; text: string } {
  switch (type) {
    case 'rate':
      return { bg: 'bg-blue-500/15', text: 'text-blue-400' }
    case 'slot':
      return { bg: 'bg-emerald-500/15', text: 'text-emerald-400' }
    case 'capacity':
      return { bg: 'bg-amber-500/15', text: 'text-amber-400' }
    case 'usage':
      return { bg: 'bg-violet-500/15', text: 'text-violet-400' }
    case 'oneTime':
      return { bg: 'bg-rose-500/15', text: 'text-rose-400' }
    case 'extraRecurring':
      return { bg: 'bg-cyan-500/15', text: 'text-cyan-400' }
    case 'credit':
      return { bg: 'bg-fuchsia-500/15', text: 'text-fuchsia-400' }
  }
}

interface ProductBrowserProps {
  currency: string
  /** The product picked: the caller prices it. */
  onSelect: (item: ProductWithPrice) => void
}

const PAGE_SIZE = 10

/** The catalog's products, searched and scrolled: one is picked to be priced. */
export const ProductBrowser = ({ currency, onSelect }: ProductBrowserProps) => {
  const [search, setSearch] = useState('')
  const [page, setPage] = useState(0)
  const [allProducts, setAllProducts] = useState<ProductWithPrice[]>([])
  const debouncedSearch = useDebounceValue(search, 300)
  const sentinelRef = useRef<HTMLDivElement>(null)

  const productsQuery = useQuery(listProductsWithPrices, {
    currency,
    query: debouncedSearch || undefined,
    pagination: { perPage: PAGE_SIZE, page },
  })

  const paginationMeta = productsQuery.data?.paginationMeta
  const hasMore = paginationMeta ? page < paginationMeta.totalPages - 1 : false

  // Reset accumulated products when search changes
  useEffect(() => {
    setPage(0)
    setAllProducts([])
  }, [debouncedSearch])

  // Append new page results
  useEffect(() => {
    const newProducts = productsQuery.data?.products
    if (!newProducts) return
    setAllProducts(prev => (page === 0 ? newProducts : [...prev, ...newProducts]))
  }, [productsQuery.data?.products, page])

  // IntersectionObserver for infinite scroll
  const loadMore = useCallback(() => {
    if (hasMore && !productsQuery.isLoading) {
      setPage(p => p + 1)
    }
  }, [hasMore, productsQuery.isLoading])

  useEffect(() => {
    const sentinel = sentinelRef.current
    if (!sentinel) return

    const observer = new IntersectionObserver(
      entries => {
        if (entries[0].isIntersecting) loadMore()
      },
      { threshold: 0.1 }
    )
    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [loadMore])

  const isEmpty = allProducts.length === 0 && !productsQuery.isLoading

  return (
    <div className="flex flex-col gap-4">
      <div className="relative">
        <SearchIcon
          size={14}
          className="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          placeholder="Search products..."
          value={search}
          onChange={e => setSearch(e.target.value)}
          className="pl-9"
        />
      </div>
      <div className="flex flex-col gap-3">
        {allProducts.map(pwp => {
          const product = pwp.product
          if (!product) return null
          return (
            <SelectableProduct
              key={product.id}
              item={pwp}
              currency={currency}
              onSelect={() => onSelect(pwp)}
            />
          )
        })}

        {/* Infinite scroll sentinel */}
        {hasMore && <div ref={sentinelRef} className="h-1" />}

        {productsQuery.isLoading && (
          <div className="flex justify-center py-4">
            <Loader2Icon size={20} className="animate-spin text-muted-foreground" />
          </div>
        )}

        {isEmpty && (
          <p className="py-8 text-center text-sm text-muted-foreground">
            {debouncedSearch ? 'No matching products' : 'No products yet'}
          </p>
        )}
      </div>
    </div>
  )
}

// --- Selectable product row ---

const SelectableProduct = ({
  item,
  currency,
  onSelect,
}: {
  item: ProductWithPrice
  currency: string
  onSelect: () => void
}) => {
  const product = item.product!
  const feeType =
    product.feeType !== undefined ? protoFeeTypeToComponentFeeType(product.feeType) : undefined
  const creditUnits = useCreditUnits()
  const badges = useMemo(
    () =>
      feeType
        ? priceSummaryBadges(feeType, item.latestPrice ?? undefined, currency, creditUnits)
        : [],
    [feeType, item.latestPrice, currency, creditUnits]
  )
  const Icon = feeType ? feeTypeIcon(feeType) : null
  const colors = feeType ? feeTypeColor(feeType) : null

  return (
    <button
      type="button"
      onClick={onSelect}
      className="flex w-full items-center gap-3 rounded-lg border border-border/60 bg-card p-3 text-left transition-colors hover:border-muted-foreground/30 hover:bg-accent/40"
    >
      {Icon && colors && (
        <div
          className={`shrink-0 w-8 h-8 rounded-md ${colors.bg} flex items-center justify-center ${colors.text}`}
        >
          {createElement(Icon, { size: 16 })}
        </div>
      )}
      <div className="flex-1 min-w-0">
        <span className="font-medium text-sm leading-tight">{product.name}</span>
        {product.description && (
          <p className="text-xs text-muted-foreground mt-0.5 line-clamp-1">{product.description}</p>
        )}
      </div>
      <div className="flex shrink-0 flex-wrap justify-end gap-1.5">
        {badges.map(b => (
          <Badge
            key={b}
            variant="outline"
            size="sm"
            className="text-[11px] font-medium text-muted-foreground"
          >
            {b}
          </Badge>
        ))}
      </div>
    </button>
  )
}
