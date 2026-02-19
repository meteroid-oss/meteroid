import { Badge, Input } from '@md/ui'
import { ChevronDownIcon, ChevronUpIcon, Loader2Icon, SearchIcon } from 'lucide-react'
import { createElement, useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { feeTypeIcon, priceSummaryBadges } from '@/features/plans/pricecomponents/utils'
import type { ComponentFeeType } from '@/features/pricing/conversions'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import type { FeeStructure } from '@/rpc/api/prices/v1/models_pb'
import { FeeStructure_UsageModel, FeeType } from '@/rpc/api/prices/v1/models_pb'
import type { ProductWithPrice } from '@/rpc/api/products/v1/models_pb'
import { listProductsWithPrices } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import { ProductPricingForm, type StructuralInfo } from './ProductPricingForm'

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
  }
}

function protoFeeTypeToComponentFeeType(feeType: FeeType): ComponentFeeType {
  switch (feeType) {
    case FeeType.RATE:
      return 'rate'
    case FeeType.SLOT:
      return 'slot'
    case FeeType.CAPACITY:
      return 'capacity'
    case FeeType.USAGE:
      return 'usage'
    case FeeType.EXTRA_RECURRING:
      return 'extraRecurring'
    case FeeType.ONE_TIME:
      return 'oneTime'
  }
}

interface ProductBrowserProps {
  currency: string
  onAdd: (data: {
    productId: string
    componentName: string
    formData: Record<string, unknown>
    feeType: ComponentFeeType
  }) => void
  submitLabel?: string
}

const PAGE_SIZE = 10

export const ProductBrowser = ({ currency, onAdd, submitLabel }: ProductBrowserProps) => {
  const [search, setSearch] = useState('')
  const [expandedId, setExpandedId] = useState<string | null>(null)
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
            <ProductCard
              key={product.id}
              item={pwp}
              isExpanded={expandedId === product.id}
              onToggle={() => setExpandedId(expandedId === product.id ? null : product.id)}
              currency={currency}
              onAdd={onAdd}
              submitLabel={submitLabel}
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

// --- Product Card ---

interface ProductCardProps {
  item: ProductWithPrice
  isExpanded: boolean
  onToggle: () => void
  currency: string
  onAdd: ProductBrowserProps['onAdd']
  submitLabel?: string
}

const ProductCard = ({
  item,
  isExpanded,
  onToggle,
  currency,
  onAdd,
  submitLabel,
}: ProductCardProps) => {
  const product = item.product!
  const feeType =
    product.feeType !== undefined ? protoFeeTypeToComponentFeeType(product.feeType) : undefined

  const structural = useMemo(
    () => extractStructuralInfo(feeType, product.feeStructure),
    [feeType, product.feeStructure]
  )

  const badges = useMemo(
    () => (feeType ? priceSummaryBadges(feeType, item.latestPrice ?? undefined, currency) : []),
    [feeType, item.latestPrice, currency]
  )

  const Icon = feeType ? feeTypeIcon(feeType) : null
  const colors = feeType ? feeTypeColor(feeType) : null

  return (
    <div
      className={`rounded-lg border overflow-hidden transition-colors ${isExpanded ? 'border-brand/40 bg-card' : 'border-border/60 bg-card hover:border-muted-foreground/30'}`}
    >
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-3.5 p-4 text-left"
      >
        {Icon && colors && (
          <div
            className={`shrink-0 w-10 h-10 rounded-lg ${colors.bg} flex items-center justify-center ${colors.text}`}
          >
            {createElement(Icon, { size: 20 })}
          </div>
        )}

        <div className="flex-1 min-w-0">
          <span className="font-semibold text-sm leading-tight">{product.name}</span>
          {product.description && (
            <p
              className={`text-xs text-muted-foreground mt-0.5 ${isExpanded ? '' : 'line-clamp-1'}`}
            >
              {product.description}
            </p>
          )}
          {!isExpanded && badges.length > 0 && (
            <div className="flex flex-wrap gap-1.5 mt-2">
              {badges.map(b => (
                <Badge
                  key={b}
                  variant="outline"
                  size="sm"
                  className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground"
                >
                  {b}
                </Badge>
              ))}
            </div>
          )}
        </div>

        <span className="shrink-0 text-muted-foreground">
          {isExpanded ? <ChevronUpIcon size={16} /> : <ChevronDownIcon size={16} />}
        </span>
      </button>

      {isExpanded && feeType && (
        <div className="border-t border-border/60 px-4 pb-4 pt-4">
          <ProductPricingForm
            feeType={feeType}
            currency={currency}
            existingPrice={item.latestPrice ?? undefined}
            structuralInfo={structural}
            onSubmit={formData => {
              // For usage library products, inject usageModel so downstream mapping
              // (toPricingTypeFromFeeType) resolves the correct pricing type
              const enrichedFormData =
                feeType === 'usage' && structural.usageModel
                  ? { ...formData, usageModel: structural.usageModel }
                  : formData
              onAdd({ productId: product.id, componentName: product.name, formData: enrichedFormData, feeType })
            }}
            submitLabel={submitLabel}
          />
        </div>
      )}
    </div>
  )
}

// --- Structural info extraction (properly typed) ---

export function extractStructuralInfo(
  feeType: ComponentFeeType | undefined,
  feeStructure: FeeStructure | undefined
): StructuralInfo {
  const structure = feeStructure?.structure
  if (!structure || !structure.case) return {}

  switch (feeType) {
    case 'slot':
      return structure.case === 'slot' ? { slotUnitName: structure.value.unitName } : {}
    case 'capacity':
      return structure.case === 'capacity' ? { metricId: structure.value.metricId } : {}
    case 'usage': {
      if (structure.case !== 'usage') return {}
      const usageModelMap: Record<number, string> = {
        [FeeStructure_UsageModel.PER_UNIT]: 'per_unit',
        [FeeStructure_UsageModel.TIERED]: 'tiered',
        [FeeStructure_UsageModel.VOLUME]: 'volume',
        [FeeStructure_UsageModel.PACKAGE]: 'package',
        [FeeStructure_UsageModel.MATRIX]: 'matrix',
      }
      return {
        metricId: structure.value.metricId,
        usageModel: usageModelMap[structure.value.model ?? 0],
      }
    }
    case 'extraRecurring':
      return structure.case === 'extraRecurring'
        ? { billingType: structure.value.billingType === 1 ? 'ADVANCE' : 'ARREAR' }
        : {}
    default:
      return {}
  }
}
