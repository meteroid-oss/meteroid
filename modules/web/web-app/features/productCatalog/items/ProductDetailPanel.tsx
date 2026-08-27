import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Separator,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { ProductEntitlementsSection } from '@/features/productCatalog/items/ProductEntitlementsSection'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { feeTypeLabel, formatCadence, formatPricingSummary } from '@/lib/mapping/prices'
import { FeeStructure_BillingType, FeeStructure_UsageModel } from '@/rpc/api/prices/v1/models_pb'
import { listPricesByProduct } from '@/rpc/api/prices/v1/prices-PricesService_connectquery'
import { getProduct, updateProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { listTaxCategories } from '@/rpc/api/taxes/v1/taxes-TaxesService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

import { MatrixRowsSection } from './MatrixRowsSection'

interface ProductDetailPanelProps {
  productId: string | null
  onClose: () => void
}

// Radix Select forbids empty-string item values, so the "no explicit category,
// fall back to the invoicing entity default" choice uses a sentinel mapped to ''.
const ENTITY_DEFAULT_CATEGORY = '__entity_default__'

function usageModelLabel(model: FeeStructure_UsageModel): string {
  switch (model) {
    case FeeStructure_UsageModel.PER_UNIT:
      return 'Per Unit'
    case FeeStructure_UsageModel.TIERED:
      return 'Tiered'
    case FeeStructure_UsageModel.VOLUME:
      return 'Volume'
    case FeeStructure_UsageModel.PACKAGE:
      return 'Package'
    case FeeStructure_UsageModel.MATRIX:
      return 'Matrix'
    default:
      return 'Unknown'
  }
}

function billingTypeLabel(bt: FeeStructure_BillingType): string {
  switch (bt) {
    case FeeStructure_BillingType.ARREAR:
      return 'Arrear (Postpaid)'
    case FeeStructure_BillingType.ADVANCE:
      return 'Advance (Prepaid)'
    default:
      return 'Unknown'
  }
}

export const ProductDetailPanel = ({ productId, onClose }: ProductDetailPanelProps) => {
  const productQuery = useQuery(
    getProduct,
    productId ? { productId } : skipToken
  )

  const pricesQuery = useQuery(
    listPricesByProduct,
    productId ? { productId } : skipToken
  )

  const queryClient = useQueryClient()
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

  const product = productQuery.data?.product
  const metricName = productQuery.data?.metricName
  const currencies = productQuery.data?.currencies ?? []
  const prices = pricesQuery.data?.prices ?? []
  const isLoading = productQuery.isLoading || pricesQuery.isLoading

  return (
    <Sheet open={!!productId} onOpenChange={() => onClose()}>
      <SheetContent size="medium">
        <SheetHeader className="pb-2">
          <SheetTitle>Product Details</SheetTitle>
          <Separator />
        </SheetHeader>

        {isLoading && (
          <div className="flex flex-col gap-4 py-4">
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-64" />
            <Skeleton className="h-4 w-32" />
            <Skeleton className="h-20 w-full" />
          </div>
        )}

        {product && !isLoading && (
          <div className="flex flex-col gap-6 py-4">
            <section className="flex flex-col gap-3">
              <h3 className="text-sm font-medium text-muted-foreground">Basic Information</h3>
              <div className="flex flex-col gap-2">
                <DetailRow label="Name" value={product.name} />
                <DetailRow label="Local ID" value={product.localId} mono />
                {product.description && (
                  <DetailRow label="Description" value={product.description} />
                )}
                <DetailRow
                  label="Fee Type"
                  value={
                    product.feeType !== undefined ? (
                      <Badge variant="secondary">{feeTypeLabel(product.feeType)}</Badge>
                    ) : (
                      <span className="text-muted-foreground">-</span>
                    )
                  }
                />
                <DetailRow
                  label="Tax category"
                  value={
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
                        {(taxCategoriesQuery.data?.taxCategories ?? []).map(c => (
                          <SelectItem key={c.id} value={c.id}>
                            {c.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  }
                />
                {product.createdAt && (
                  <DetailRow
                    label="Created"
                    value={parseAndFormatDate(product.createdAt)}
                  />
                )}
              </div>
            </section>

            {product.feeStructure?.structure.case && (
              <>
                <Separator />
                <section className="flex flex-col gap-3">
                  <h3 className="text-sm font-medium text-muted-foreground">Fee Structure</h3>
                  <FeeStructureDetails
                    structure={product.feeStructure.structure}
                    metricName={metricName}
                  />
                </section>
              </>
            )}

            {product.feeStructure?.structure.case === 'usage' &&
              product.feeStructure.structure.value.model ===
                FeeStructure_UsageModel.MATRIX && (
                <>
                  <Separator />
                  <MatrixRowsSection
                    productId={productId!}
                    metricId={product.feeStructure.structure.value.metricId}
                    currencies={currencies}
                  />
                </>
              )}

            {env.entitlementsEnabled && (
              <>
                <Separator />
                <section className="flex flex-col gap-3">
                  <h3 className="text-sm font-medium text-muted-foreground">Entitlements</h3>
                  <ProductEntitlementsSection productId={product.id} />
                </section>
              </>
            )}

            <Separator />
            <section className="flex flex-col gap-3">
              <h3 className="text-sm font-medium text-muted-foreground">
                Prices ({prices.length})
              </h3>
              {prices.length === 0 ? (
                <p className="text-sm text-muted-foreground">No prices defined for this product.</p>
              ) : (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Cadence</TableHead>
                      <TableHead>Currency</TableHead>
                      <TableHead>Pricing</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {prices.map(price => (
                      <TableRow key={price.id}>
                        <TableCell>{formatCadence(price.cadence)}</TableCell>
                        <TableCell>
                          <span className="font-mono text-xs">{price.currency.toUpperCase()}</span>
                        </TableCell>
                        <TableCell>{formatPricingSummary(price)}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              )}
            </section>
          </div>
        )}
      </SheetContent>
    </Sheet>
  )
}

function DetailRow({
  label,
  value,
  mono,
}: {
  label: string
  value: React.ReactNode
  mono?: boolean
}) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="text-sm text-muted-foreground w-28 shrink-0">{label}</span>
      <span className={`text-sm ${mono ? 'font-mono' : ''}`}>{value}</span>
    </div>
  )
}

function MetricLink({ metricId, metricName }: { metricId: string; metricName?: string }) {
  const basePath = useBasePath()
  return (
    <Link to={`${basePath}/metrics/${metricId}`} className="text-sm text-primary hover:underline">
      {metricName ?? metricId}
    </Link>
  )
}

function FeeStructureDetails({
  structure,
  metricName,
}: {
  structure: NonNullable<import('@/rpc/api/prices/v1/models_pb').FeeStructure['structure']>
  metricName?: string
}) {
  switch (structure.case) {
    case 'rate':
      return <p className="text-sm text-muted-foreground">Flat rate pricing with no additional structure parameters.</p>
    case 'slot':
      return (
        <div className="flex flex-col gap-2">
          <DetailRow label="Unit Name" value={structure.value.unitName} />
        </div>
      )
    case 'capacity':
      return (
        <div className="flex flex-col gap-2">
          <DetailRow label="Metric" value={<MetricLink metricId={structure.value.metricId} metricName={metricName} />} />
        </div>
      )
    case 'usage':
      return (
        <div className="flex flex-col gap-2">
          <DetailRow label="Metric" value={<MetricLink metricId={structure.value.metricId} metricName={metricName} />} />
          <DetailRow label="Model" value={usageModelLabel(structure.value.model)} />
        </div>
      )
    case 'extraRecurring':
      return (
        <div className="flex flex-col gap-2">
          <DetailRow label="Billing Type" value={billingTypeLabel(structure.value.billingType)} />
        </div>
      )
    case 'oneTime':
      return <p className="text-sm text-muted-foreground">One-time charge with no additional structure parameters.</p>
    default:
      return null
  }
}
