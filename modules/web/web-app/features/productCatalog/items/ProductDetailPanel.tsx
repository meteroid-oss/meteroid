import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ArchiveIcon, ArchiveRestoreIcon, CopyIcon, MoreHorizontal } from 'lucide-react'
import { Fragment, type ReactNode } from 'react'
import { Link } from 'react-router'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { ProductAppearancePicker } from '@/features/productCatalog/items/ProductAppearancePicker'
import { ProductEntitlementsSection } from '@/features/productCatalog/items/ProductEntitlementsSection'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'
import { ProductSeatsSection } from '@/features/productCatalog/items/ProductSeatsSection'
import { ProductUsedInSection } from '@/features/productCatalog/items/ProductUsedInSection'
import { useProductArchive } from '@/features/productCatalog/items/useProductArchive'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { copyToClipboard } from '@/lib/helpers'
import { FeeStructure_UsageModel } from '@/rpc/api/prices/v1/models_pb'
import {
  getProduct,
  listProductUsage,
  updateProduct,
} from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { listTaxCategories } from '@/rpc/api/taxes/v1/taxes-TaxesService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

import { MatrixRowsSection } from './MatrixRowsSection'
import { chargeKindsWords, chargesForWords, colorLabel, iconLabel } from './productWords'

import type { ProductAppearance } from '@/features/productCatalog/items/ProductIcon'
import type { FeeStructure } from '@/rpc/api/prices/v1/models_pb'
import type { Product } from '@/rpc/api/products/v1/models_pb'

interface ProductDetailPanelProps {
  productId: string | null
  onClose: () => void
}

// Radix Select forbids empty-string item values, so the "no explicit category,
// fall back to the invoicing entity default" choice uses a sentinel mapped to ''.
const ENTITY_DEFAULT_CATEGORY = '__entity_default__'

export const ProductDetailPanel = ({ productId, onClose }: ProductDetailPanelProps) => {
  const productQuery = useQuery(getProduct, productId ? { productId } : skipToken)

  const queryClient = useQueryClient()
  const productArchive = useProductArchive()
  const taxCategoriesQuery = useQuery(listTaxCategories, {})
  const updateProductMut = useMutation(updateProduct, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getProduct,
          input: { productId: productId ?? '' },
          cardinality: 'finite',
        }),
      })
      toast.success('Tax category updated')
    },
    onError: e => toast.error(`Failed to update tax category: ${e.message}`),
  })
  // Pending until the product is read again, so the picker shows the pick meanwhile.
  const appearanceMut = useMutation(updateProduct, {
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: updateProduct.parent, cardinality: undefined }),
      }),
    onError: e => toast.error(`Failed to update the product's icon: ${e.message}`),
  })

  const product = productQuery.data?.product
  const metricName = productQuery.data?.metricName
  const currencies = productQuery.data?.currencies ?? []
  const picked = appearanceMut.isPending ? appearanceMut.variables : undefined
  const appearance: ProductAppearance = picked
    ? { icon: picked.icon || undefined, color: picked.color || undefined }
    : { icon: product?.icon, color: product?.color }
  const taxCategories = taxCategoriesQuery.data?.taxCategories ?? []
  const structure = product?.feeStructure?.structure
  // How the plan charges using it bill it: a product holds no structure of its own.
  const usageQuery = useQuery(listProductUsage, productId ? { productId } : skipToken)
  const charged: FeeStructure[] = (usageQuery.data?.plans ?? []).flatMap(usage =>
    usage.components.flatMap(c => (c.feeStructure ? [c.feeStructure] : []))
  )
  if (product?.feeStructure) charged.push(product.feeStructure)
  // The quantities its seat charges count, each with its own rules.
  const seatQuantities = [
    ...new Set(
      charged.flatMap(s =>
        s.structure.case === 'slot' && s.structure.value.quantityId
          ? [s.structure.value.quantityId]
          : []
      )
    ),
  ]

  return (
    <Sheet open={!!productId} onOpenChange={() => onClose()}>
      <SheetContent size="medium">
        <SheetHeader className="space-y-0 pb-2 pr-8">
          <div className="flex items-start gap-3">
            <ProductIcon {...appearance} className="mt-0.5" />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <SheetTitle className="truncate">{product?.name ?? 'Product'}</SheetTitle>
                {product?.archivedAt && (
                  <Badge variant="secondary">
                    Archived {parseAndFormatDate(product.archivedAt)}
                  </Badge>
                )}
              </div>
              {product && (
                <SheetDescription>
                  <ChargesForLine product={product} metricName={metricName} charged={charged} />
                </SheetDescription>
              )}
            </div>
            {product && (
              <Gate perm="catalog:manage">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-7 shrink-0"
                      aria-label="Product actions"
                    >
                      <MoreHorizontal size={16} />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    {product.archivedAt ? (
                      <DropdownMenuItem
                        disabled={productArchive.isPending}
                        onSelect={() => productArchive.unarchive(product.id)}
                      >
                        <ArchiveRestoreIcon size={14} className="mr-2" /> Unarchive
                      </DropdownMenuItem>
                    ) : (
                      <DropdownMenuItem
                        disabled={productArchive.isPending}
                        onSelect={() => productArchive.archive(product.id)}
                      >
                        <ArchiveIcon size={14} className="mr-2" /> Archive
                      </DropdownMenuItem>
                    )}
                  </DropdownMenuContent>
                </DropdownMenu>
              </Gate>
            )}
          </div>
          {product?.description && (
            <p className="pt-2 text-sm text-muted-foreground">{product.description}</p>
          )}
        </SheetHeader>

        {productQuery.isLoading && (
          <div className="flex flex-col gap-4 py-4">
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-64" />
            <Skeleton className="h-4 w-32" />
            <Skeleton className="h-20 w-full" />
          </div>
        )}

        {product && !productQuery.isLoading && (
          <div className="flex flex-col gap-8 py-4">
            <dl className="grid grid-cols-[7rem_minmax(0,1fr)] items-center gap-x-4 gap-y-3 text-sm">
              <Property label="Appearance">
                <div className="flex items-center gap-2">
                  <ProductIcon {...appearance} size="sm" />
                  <span>
                    {iconLabel(appearance.icon)} · {colorLabel(appearance.color)}
                  </span>
                  <Gate perm="catalog:manage">
                    <ProductAppearancePicker
                      {...appearance}
                      onChange={next =>
                        appearanceMut.mutate({
                          productId: product.id,
                          name: product.name,
                          icon: next.icon ?? '',
                          color: next.color ?? '',
                        })
                      }
                    >
                      <Button type="button" variant="link" size="sm" className="mt-0 h-auto px-1">
                        Change
                      </Button>
                    </ProductAppearancePicker>
                  </Gate>
                </div>
              </Property>
              <Property label="Tax category">
                <Gate
                  perm="catalog:manage"
                  fallback={
                    taxCategories.find(c => c.id === product.taxCategoryId)?.name ??
                    'Entity default'
                  }
                >
                  <Select
                    value={product.taxCategoryId || ENTITY_DEFAULT_CATEGORY}
                    disabled={updateProductMut.isPending}
                    onValueChange={v =>
                      updateProductMut.mutate({
                        productId: product.id,
                        name: product.name,
                        taxCategoryId: v === ENTITY_DEFAULT_CATEGORY ? '' : v,
                      })
                    }
                  >
                    <SelectTrigger className="h-8 w-[240px] text-sm">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={ENTITY_DEFAULT_CATEGORY}>Entity default</SelectItem>
                      {taxCategories.map(c => (
                        <SelectItem key={c.id} value={c.id}>
                          {c.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </Gate>
              </Property>
              <Property label="API handle">
                <button
                  type="button"
                  title="Copy"
                  onClick={() =>
                    copyToClipboard(product.localId, () => toast.success('Copied to clipboard'))
                  }
                  className="group inline-flex max-w-full items-center gap-1.5 font-mono text-xs text-muted-foreground hover:text-foreground"
                >
                  <span className="truncate">{product.localId}</span>
                  <CopyIcon size={11} className="shrink-0 opacity-60 group-hover:opacity-100" />
                </button>
              </Property>
              {product.createdAt && (
                <Property label="Created">{parseAndFormatDate(product.createdAt)}</Property>
              )}
            </dl>

            {seatQuantities.length > 0 && (
              <section className="flex flex-col gap-3">
                <div className="flex flex-wrap items-baseline gap-x-2">
                  <h3 className="text-sm font-semibold">Seats</h3>
                  <span className="text-xs text-muted-foreground">
                    When a change of seats takes effect
                  </span>
                </div>
                <ProductSeatsSection quantityIds={seatQuantities} />
              </section>
            )}

            {structure?.case === 'usage' &&
              (structure.value.model === FeeStructure_UsageModel.DIMENSION_RATES ||
                structure.value.model === FeeStructure_UsageModel.MATRIX) && (
                <MatrixRowsSection
                  productId={product.id}
                  metricId={structure.value.metricId}
                  currencies={currencies}
                />
              )}

            {env.entitlementsEnabled && <ProductEntitlementsSection productId={product.id} />}

            <ProductUsedInSection product={product} />
          </div>
        )}
      </SheetContent>
    </Sheet>
  )
}

const Property = ({ label, children }: { label: string; children: ReactNode }) => (
  <>
    <dt className="text-muted-foreground">{label}</dt>
    <dd className="min-w-0">{children}</dd>
  </>
)

/**
 * "Seats · counts seat", "Usage · Tokens · tiered": what its charges bill it for, the metric a
 * link; "Flat fee or seats" when they bill it several ways.
 */
const ChargesForLine = ({
  product,
  metricName,
  charged,
}: {
  product: Product
  metricName?: string
  charged: FeeStructure[]
}) => {
  const basePath = useBasePath()
  const kinds = product.feeStructure ? '' : chargeKindsWords(charged)
  const words = product.feeStructure
    ? chargesForWords(product.feeStructure, metricName)
    : kinds
      ? [kinds]
      : ['Not charged yet']
  return (
    <span>
      {words.map((word, i) => (
        <Fragment key={i}>
          {i > 0 && ' · '}
          {typeof word === 'string' ? (
            word
          ) : (
            <Link to={`${basePath}/metrics/${word.metricId}`} className="hover:underline">
              {word.name}
            </Link>
          )}
        </Fragment>
      ))}
    </span>
  )
}
