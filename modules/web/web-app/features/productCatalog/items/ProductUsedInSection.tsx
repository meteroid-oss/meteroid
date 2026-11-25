import { Button, Skeleton, cn } from '@md/ui'
import { Link } from 'react-router'

import { metricUnitFromProto } from '@/features/billing/metrics/units'
import { PriceChip } from '@/features/plans/canvas/ChargeRow'
import { useComponentMetric } from '@/features/plans/pricecomponents/componentRefs'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { listProductUsage } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import { chargeLines, gapLabel, linesKey } from './usedIn'

import type { UsedInLine } from './usedIn'
import type { ChargeContext } from '@/features/plans/canvas/billingWords'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { Product } from '@/rpc/api/products/v1/models_pb'
import type { ProductAddOnUsage, ProductPlanUsage } from '@/rpc/api/products/v1/products_pb'

const chip = 'inline-flex items-baseline gap-1.5 rounded-md border px-2.5 py-1'

const GapChip = ({ label }: { label: string }) => (
  <span className={cn(chip, 'border-dashed border-warning/60 py-0.5 text-xs text-warning')}>
    {label}
  </span>
)

const noop = () => {}

/** One currency: its code, then its cells' chips; a cell without a price is a gap. */
const CurrencyLine = ({ line, ctx }: { line: UsedInLine; ctx: ChargeContext }) => (
  <div className="flex items-start gap-3">
    <span className="w-9 shrink-0 pt-1.5 font-mono text-xs text-muted-foreground">
      {line.currency}
    </span>
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
      {line.cells.map((cell, i) =>
        cell.prices.length === 0 ? (
          <GapChip key={i} label={gapLabel(cell)} />
        ) : (
          <span key={i} className="inline-flex items-center gap-1.5">
            {cell.option && <span className="text-xs text-muted-foreground">{cell.option}</span>}
            <PriceChip
              prices={cell.prices}
              currency={line.currency}
              missing={gapLabel(cell)}
              ctx={ctx}
              isDraft={false}
              onEdit={noop}
            />
          </span>
        )
      )}
    </div>
  </div>
)

/** The version's charges of the product: an included one says so, a priced one its cells. */
const VersionPrices = ({ usage, ctx }: { usage: ProductPlanUsage; ctx: ChargeContext }) => (
  <div className="space-y-2">
    {usage.components.map(component => (
      <div key={component.id} className="space-y-1.5">
        {usage.components.length > 1 && (
          <p className="text-xs font-medium text-muted-foreground">{component.name}</p>
        )}
        {component.prices.length === 0 ? (
          <span className={cn(chip, 'border-success/30 bg-success/10 py-0.5 text-xs text-success')}>
            Included in the plan
          </span>
        ) : (
          chargeLines(component, usage.options, usage.currencies).map(line => (
            <CurrencyLine key={line.currency} line={line} ctx={ctx} />
          ))
        )}
      </div>
    ))}
  </div>
)

const pricesKey = (usage: ProductPlanUsage, ctx: ChargeContext) =>
  usage.components
    .map(c => `${c.name}|${linesKey(chargeLines(c, usage.options, usage.currencies), ctx)}`)
    .join('#')

const DraftTag = ({ version }: { version: number }) => (
  <span className="rounded bg-warning/15 px-1.5 py-0.5 text-xs font-medium text-warning">
    Draft v{version}
  </span>
)

/** A plan selling the product: its live version's prices, and its draft's when they differ. */
const PlanRow = ({
  active,
  draft,
  ctx,
}: {
  active?: ProductPlanUsage
  draft?: ProductPlanUsage
  ctx: ChargeContext
}) => {
  const basePath = useBasePath()
  const lead = (active ?? draft)!
  const planPath = `${basePath}/plans/${lead.planId}`
  const draftDiffers = !!active && !!draft && pricesKey(active, ctx) !== pricesKey(draft, ctx)
  return (
    <div className="space-y-2.5 px-4 py-3">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <Link
          to={active ? planPath : `${planPath}/draft`}
          className="text-sm font-semibold text-foreground hover:underline"
        >
          {lead.planName}
        </Link>
        {active && <span className="text-xs text-muted-foreground">v{active.version}</span>}
        {draft && (
          <Link to={`${planPath}/draft`} className="inline-flex items-center gap-1.5">
            <DraftTag version={draft.version} />
            {active && !draftDiffers && (
              <span className="text-xs text-muted-foreground">same prices</span>
            )}
            {!active && <span className="text-xs text-muted-foreground">not published</span>}
          </Link>
        )}
      </div>
      <VersionPrices usage={lead} ctx={ctx} />
      {draftDiffers && (
        <div className="space-y-2 border-t border-dashed border-border pt-2.5">
          <p className="text-xs text-muted-foreground">
            In <DraftTag version={draft!.version} />
          </p>
          <VersionPrices usage={draft!} ctx={ctx} />
        </div>
      )}
    </div>
  )
}

/** An add-on of the product: its prices by currency, each with its billing periods. */
const AddOnRow = ({ addOn, ctx }: { addOn: ProductAddOnUsage; ctx: ChargeContext }) => {
  const basePath = useBasePath()
  const byCurrency = new Map<string, Price[]>()
  for (const price of addOn.prices) {
    byCurrency.set(price.currency, [...(byCurrency.get(price.currency) ?? []), price])
  }
  const lines: UsedInLine[] = [...byCurrency.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([currency, prices]) => ({ currency, cells: [{ prices }] }))
  return (
    <div className="space-y-2.5 px-4 py-3">
      <div className="flex items-center gap-2">
        <Link
          to={`${basePath}/addons/${addOn.addOnId}`}
          className="text-sm font-semibold text-foreground hover:underline"
        >
          {addOn.name}
        </Link>
        <span className="text-xs text-muted-foreground">Add-on</span>
      </div>
      {lines.length === 0 ? (
        <GapChip label="No price" />
      ) : (
        lines.map(line => <CurrencyLine key={line.currency} line={line} ctx={ctx} />)
      )}
    </div>
  )
}

/** Where the product is sold, read-only: each plan's price of it, and the add-ons selling it. */
export const ProductUsedInSection = ({ product }: { product: Product }) => {
  const usageQuery = useQuery(listProductUsage, { productId: product.id })
  const structure = product.feeStructure?.structure
  const metricId =
    structure?.case === 'usage' || structure?.case === 'capacity'
      ? structure.value.metricId
      : undefined
  const metric = useComponentMetric(metricId)
  const ctx: ChargeContext = { product, metric, unit: metricUnitFromProto(metric) }

  // A plan's active version and its draft read as one row.
  const plans = new Map<string, { active?: ProductPlanUsage; draft?: ProductPlanUsage }>()
  for (const usage of usageQuery.data?.plans ?? []) {
    const row = plans.get(usage.planId) ?? {}
    row[usage.isDraft ? 'draft' : 'active'] = usage
    plans.set(usage.planId, row)
  }
  const addOns = usageQuery.data?.addOns ?? []

  return (
    <section className="flex flex-col gap-3">
      <div className="flex flex-wrap items-baseline gap-x-2">
        <h3 className="text-sm font-semibold">Used in</h3>
        <span className="text-xs text-muted-foreground">Prices are set in each plan</span>
      </div>
      {usageQuery.isLoading ? (
        <Skeleton className="h-16 w-full" />
      ) : usageQuery.isError ? (
        <div className="flex items-center gap-3" role="alert">
          <p className="text-sm text-destructive">
            Where this product is used could not be loaded.
          </p>
          <Button variant="secondary" size="sm" onClick={() => void usageQuery.refetch()}>
            Retry
          </Button>
        </div>
      ) : (
        <>
          {plans.size === 0 ? (
            <p className="text-sm text-muted-foreground">Not used in any plan.</p>
          ) : (
            <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
              {[...plans.entries()].map(([planId, row]) => (
                <PlanRow key={planId} active={row.active} draft={row.draft} ctx={ctx} />
              ))}
            </div>
          )}
          {addOns.length > 0 && (
            <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
              {addOns.map(addOn => (
                <AddOnRow key={addOn.addOnId} addOn={addOn} ctx={ctx} />
              ))}
            </div>
          )}
        </>
      )}
    </section>
  )
}
