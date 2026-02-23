import { disableQuery } from '@connectrpc/connect-query'
import {
  Badge,
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
import { Link } from 'react-router-dom'

import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { feeTypeLabel, formatCadence, formatPricingSummary } from '@/lib/mapping/prices'
import { FeeStructure_BillingType, FeeStructure_UsageModel } from '@/rpc/api/prices/v1/models_pb'
import { listPricesByProduct } from '@/rpc/api/prices/v1/prices-PricesService_connectquery'
import { getProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

import { MatrixRowsSection } from './MatrixRowsSection'

interface ProductDetailPanelProps {
  productId: string | null
  onClose: () => void
}

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
    productId ? { productId } : disableQuery
  )

  const pricesQuery = useQuery(
    listPricesByProduct,
    productId ? { productId } : disableQuery
  )

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
