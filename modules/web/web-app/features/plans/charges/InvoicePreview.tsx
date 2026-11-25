import { skipToken } from '@connectrpc/connect-query'
import { Slider, cn } from '@md/ui'
import Decimal from 'decimal.js'
import { MinusIcon, PlusIcon } from 'lucide-react'
import { useState, type ReactNode } from 'react'

import { formatPerUnit, unitPlural } from '@/features/billing/metrics/units'
import { cardRatesForPrice } from '@/features/productCatalog/ratecards/bridge'
import { dimensionName, unitSizeLabel } from '@/features/productCatalog/ratecards/rateTable'
import { useQuery } from '@/lib/connectrpc'
import { getRateCard } from '@/rpc/api/ratecards/v1/ratecards-RateCardsService_connectquery'

import { Segmented } from './ChargeComposer'
import { normalizeShape } from './compile'
import { sortedLevels } from './kinds'
import { billsOnce, invoicePreview, sampleRange, timingCaption } from './preview'
import { formatMoney } from './summary'

import type { ChargeShape, ChargeValues } from './compile'
import type { LevelRow } from './kinds'
import type { PreviewContext, PreviewLine } from './preview'
import type { DimensionRateRow } from '@/features/pricing/schemas'

// "On the invoice": the line(s) a charge produces, at a quantity the operator sets beside them.
// Computed here as the form changes, nothing asked of the server but a rate card's rates.

const grouped = (value: unknown) => {
  const raw = String(value ?? '')
  if (raw === '') return ''
  const n = Number(raw)
  return Number.isFinite(n) ? n.toLocaleString('en-US') : raw
}
const digitsOnly = (raw: string) => raw.replace(/[^\d]/g, '')
const compact = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })

const cents = (amount: Decimal, currency: string) =>
  formatMoney(amount.toDecimalPlaces(2, Decimal.ROUND_HALF_UP).toString(), currency)

/** The column's frame: its title, the example's control, the lines, a total and when it bills. */
export const PreviewFrame = ({
  control,
  total,
  caption,
  children,
}: {
  control?: ReactNode
  total?: { label: string; amount: string }
  caption?: string
  children: ReactNode
}) => (
  <aside
    aria-label="On the invoice"
    className="flex flex-col overflow-hidden rounded-lg border border-border bg-background @3xl:min-h-[22rem]"
  >
    <p className="border-b border-border px-4 py-2.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
      On the invoice
    </p>
    {control && <div className="space-y-2.5 border-b border-border px-4 py-3">{control}</div>}
    <div className="flex-1 px-4 py-3">{children}</div>
    {total && (
      <div className="flex items-baseline justify-between gap-3 border-t border-border px-4 py-3">
        <span className="text-xs text-muted-foreground">{total.label}</span>
        <span className="font-mono text-xl font-semibold tabular-nums text-foreground">
          {total.amount}
        </span>
      </div>
    )}
    {caption && (
      <p className="border-t border-dashed border-border px-4 py-2 text-[11px] text-muted-foreground">
        {caption}
      </p>
    )}
  </aside>
)

const Lines = ({ lines, money }: { lines: PreviewLine[]; money: (amount: Decimal) => string }) => (
  <ul className="space-y-2">
    {lines.map((line, i) => (
      <li
        key={i}
        className={cn(
          'flex items-baseline justify-between gap-3',
          line.sub && 'ml-1 border-l border-border pl-3'
        )}
      >
        <span className="min-w-0">
          <span
            className={cn(
              'block',
              line.sub ? 'text-xs text-foreground' : 'text-sm font-medium text-foreground'
            )}
          >
            {line.label}
          </span>
          {line.detail && (
            <span className="block text-[11px] text-muted-foreground">{line.detail}</span>
          )}
        </span>
        <span
          className={cn(
            'shrink-0 font-mono tabular-nums',
            line.sub ? 'text-xs text-muted-foreground' : 'text-sm text-foreground'
          )}
        >
          {money(line.amount)}
        </span>
      </li>
    ))}
  </ul>
)

/** Before a kind is picked, or a product included with no price. */
export const PreviewNote = ({ name, included }: { name?: string; included?: boolean }) => (
  <PreviewFrame caption={included ? 'Nothing is billed' : undefined}>
    {included ? (
      <Lines
        lines={[
          {
            label: name || 'This product',
            detail: 'Its features come with the plan',
            amount: new Decimal(0),
          },
        ]}
        money={() => 'Included'}
      />
    ) : (
      <p className="text-xs text-muted-foreground">
        Pick what it charges for: the line it puts on an invoice shows here.
      </p>
    )}
  </PreviewFrame>
)

export interface InvoicePreviewProps extends Omit<PreviewContext, 'name'> {
  shape: ChargeShape
  values: ChargeValues
  name?: string
}

/** The charge's invoice line(s) at a sample quantity: seats, usage, a level picked. */
export const InvoicePreview = ({ shape, values, name, ...ctx }: InvoicePreviewProps) => {
  const s = normalizeShape(shape)
  const range = sampleRange(s, values, ctx.unit)
  // Around what the price sets (its included amount, its tiers) until the operator sets one.
  const [typed, setQuantity] = useState<string>()
  const quantity = typed ?? String(range.initial)
  const [level, setLevel] = useState(0)
  const [later, setLater] = useState(false)
  const context: PreviewContext = { ...ctx, name: name || 'This charge' }
  const caption = timingCaption(s, ctx)

  if (s.model === 'by_dimension') {
    return <RatesPreview values={values} context={context} caption={caption} />
  }

  const q = Number(quantity || '0')
  const preview = invoicePreview(s, values, { quantity: q, level, later }, context)
  const money = (amount: Decimal) => cents(amount, ctx.currency)
  // "calls", or "× 1M tokens" when the unit is itself a quantity.
  const priced = formatPerUnit(ctx.unit).replace(/^per /, '')
  const noun =
    s.source === 'seats'
      ? `${ctx.seat}s`
      : /^\d/.test(priced)
        ? `× ${priced}`
        : unitPlural(ctx.unit)
  const max = Math.max(range.max, q)
  const levels = sortedLevels((values.thresholds as LevelRow[] | undefined) ?? [])

  const usageControl = (
    <div className="space-y-2.5">
      <label className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
        Usage this {ctx.per}
        <input
          inputMode="numeric"
          aria-label={`Usage this ${ctx.per}`}
          className="w-28 rounded border border-input bg-transparent px-1.5 py-0.5 font-mono tabular-nums text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring"
          value={grouped(quantity)}
          onChange={e => setQuantity(digitsOnly(e.target.value))}
        />
        {noun}
      </label>
      <Slider
        aria-label={`Usage this ${ctx.per}`}
        variant="neutral"
        min={0}
        max={max}
        step={Math.max(1, Math.round(max / 200))}
        value={[Math.min(q, max)]}
        onValueChange={([next]) => setQuantity(String(next))}
      />
    </div>
  )

  const control = (() => {
    if (s.source === 'seats') {
      const step = (by: number) => setQuantity(String(Math.max(0, q + by)))
      return (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <button
            type="button"
            aria-label={`One ${ctx.seat} less`}
            onClick={() => step(-1)}
            className="grid size-7 place-items-center rounded border border-input hover:bg-accent"
          >
            <MinusIcon size={12} />
          </button>
          <input
            inputMode="numeric"
            aria-label={`Number of ${ctx.seat}s`}
            className="w-14 rounded border border-input bg-transparent px-1.5 py-1 text-center font-mono tabular-nums text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring"
            value={grouped(quantity)}
            onChange={e => setQuantity(digitsOnly(e.target.value))}
          />
          <button
            type="button"
            aria-label={`One ${ctx.seat} more`}
            onClick={() => step(1)}
            className="grid size-7 place-items-center rounded border border-input hover:bg-accent"
          >
            <PlusIcon size={12} />
          </button>
          {noun}
        </div>
      )
    }
    if (s.source === 'metered' && s.model === 'included') {
      return (
        <>
          {levels.length > 1 && (
            <Segmented<string>
              label="Level the customer picked"
              value={String(Math.min(level, levels.length - 1))}
              options={levels.map((l, i) => ({
                value: String(i),
                label: `Up to ${compact.format(Number(l.included ?? 0))}`,
              }))}
              onChange={next => setLevel(Number(next))}
            />
          )}
          {usageControl}
        </>
      )
    }
    if (s.source === 'metered') return usageControl
    // A one-time fee is on the first invoice, not on the next ones.
    if (billsOnce(s)) {
      return (
        <Segmented<'first' | 'later'>
          label="Invoice"
          value={later ? 'later' : 'first'}
          options={[
            { value: 'first', label: 'First invoice' },
            { value: 'later', label: `Each ${ctx.per} after` },
          ]}
          onChange={next => setLater(next === 'later')}
        />
      )
    }
    return undefined
  })()

  return (
    <PreviewFrame
      control={control}
      total={preview && { label: preview.totalLabel, amount: money(preview.total) }}
      caption={caption}
    >
      {preview ? (
        <Lines lines={preview.lines} money={money} />
      ) : (
        <p className="text-xs text-muted-foreground">Set its price to see it.</p>
      )}
    </PreviewFrame>
  )
}

/** Usage priced by dimension has no one quantity: its rates as the invoice applies them. */
const RatesPreview = ({
  values,
  context,
  caption,
}: {
  values: ChargeValues
  context: PreviewContext
  caption: string
}) => {
  const card = values.rateSource === 'card'
  const cardId = card ? (values.rateCardId as string | undefined) : undefined
  const rateCard = useQuery(getRateCard, cardId ? { id: cardId } : skipToken).data
  const copied =
    rateCard?.card &&
    cardRatesForPrice(rateCard.card, rateCard.rates, context.currency, values.markup as string)
  const own = {
    dimensions: (values.dimensions as string[] | undefined) ?? [],
    keyRates: ((values.keyRates as DimensionRateRow[] | undefined) ?? []).filter(row =>
      row.rate?.trim()
    ),
    unitSize: String(values.unitSize ?? '1'),
  }
  const rates = card ? copied : own
  const unit = context.unit?.singular
    ? { singular: context.unit.singular, plural: context.unit.plural || context.unit.singular }
    : undefined
  const shown = rates?.keyRates.slice(0, 6) ?? []
  const key = (patterns: string[]) =>
    patterns.every(p => p === '*')
      ? 'Anything else'
      : patterns
          .map((p, i) =>
            p === '*' ? `any ${dimensionName(rates?.dimensions[i] ?? '').toLowerCase()}` : p
          )
          .join(' · ')

  return (
    <PreviewFrame caption={caption}>
      {shown.length ? (
        <div className="space-y-2">
          <p className="text-xs text-muted-foreground">
            {context.name}: each event at its rate, {unitSizeLabel(rates?.unitSize ?? '1', unit)}
          </p>
          <ul className="space-y-1 text-xs">
            {shown.map((row, i) => (
              <li key={i} className="flex justify-between gap-3">
                <span className="truncate text-foreground">{key(row.key)}</span>
                <span className="font-mono tabular-nums text-muted-foreground">
                  {context.creditUnitName
                    ? `${row.rate} ${context.creditUnitName}`
                    : formatMoney(row.rate, context.currency)}
                </span>
              </li>
            ))}
          </ul>
          {(rates?.keyRates.length ?? 0) > shown.length && (
            <p className="text-[11px] text-muted-foreground">
              and {(rates?.keyRates.length ?? 0) - shown.length} more
            </p>
          )}
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          {card ? 'Choose a rate card with rates in this currency.' : 'Add its rates.'}
        </p>
      )}
    </PreviewFrame>
  )
}
