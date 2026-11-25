import { create } from '@bufbuild/protobuf'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
  cn,
} from '@md/ui'
import { CheckIcon, PlusIcon, XIcon } from 'lucide-react'
import { FormEvent, useState } from 'react'

import {
  isAmount,
  priceInputOf,
  priceText,
  simpleAmountOf,
  templateFor,
} from '@/features/plans/canvas/chargeFormat'
import {
  TERMS,
  cellPrices,
  hasTerm,
  orderCurrencies,
  termAdverb,
  termLabel,
} from '@/features/plans/currencies/priceCells'
import { useQuery } from '@/lib/connectrpc'
import { PriceInputSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import type { PriceInput } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

/** An add-on's prices, currencies × billing periods; a subscription sees its own currency's. */
export const AddOnPricesTable = ({
  prices,
  defaultCurrency,
}: {
  prices: Price[]
  defaultCurrency?: string
}) => {
  if (prices.length === 0) return null
  const currencies = orderCurrencies(
    prices.map(p => p.currency),
    defaultCurrency ?? prices[0].currency
  )
  const periods = TERMS.map(t => t.term).filter(term =>
    prices.some(p => hasTerm(p) && p.cadence === term)
  )
  const once = prices.some(p => !hasTerm(p))
  const columns: (BillingPeriod | undefined)[] = [...periods, ...(once ? [undefined] : [])]
  const cell = (currency: string, period: BillingPeriod | undefined) =>
    prices.find(p =>
      p.currency === currency && (period === undefined ? !hasTerm(p) : hasTerm(p) && p.cadence === period)
    )

  return (
    <div className="overflow-x-auto rounded-md border border-border">
      <table className="w-full text-sm">
        <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
          <tr>
            <th scope="col" className="px-3 py-2 font-medium">
              Currency
            </th>
            {columns.map(period => (
              <th key={period ?? 'once'} scope="col" className="px-3 py-2 font-medium">
                {period === undefined ? 'Price' : termLabel(period)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-border">
          {currencies.map(currency => (
            <tr key={currency}>
              <th scope="row" className="px-3 py-2 text-left font-mono font-medium">
                {currency}
              </th>
              {columns.map(period => {
                const price = cell(currency, period)
                const text = price && priceText(price, {})
                return (
                  <td key={period ?? 'once'} className="px-3 py-2">
                    {text ? (
                      <span className="font-mono tabular-nums">
                        {text.main}
                        {text.suffix && (
                          <span className="ml-1 text-xs text-muted-foreground">{text.suffix}</span>
                        )}
                      </span>
                    ) : (
                      <span className="text-xs text-muted-foreground">-</span>
                    )}
                  </td>
                )
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** A draft price of the grid: a Price without an id until saved. */
export const priceOfInput = (input: PriceInput): Price =>
  create(PriceSchema, {
    cadence: input.cadence,
    currency: input.currency,
    pricing: input.pricing,
    creditUnit: input.creditUnit,
  })

/** What the edit sends for a price: unchanged ones keep their id server-side. */
export const inputOfPrice = (price: Price): PriceInput =>
  create(PriceInputSchema, {
    cadence: price.cadence,
    currency: price.currency,
    pricing: price.pricing,
    creditUnit: price.creditUnit,
  })

/** Whether two prices hold one cell: one currency, on one period or either of none. */
const sameCell = (a: Price, b: Price) =>
  a.currency === b.currency && (!hasTerm(a) || !hasTerm(b) || a.cadence === b.cadence)

/** `prices` with `price` in its cell, in the place of the one it replaces; `replacing` leaves. */
export const withPrice = (prices: Price[], price: Price, replacing?: Price): Price[] => {
  const out: Price[] = []
  let placed = false
  for (const p of prices) {
    if (p !== replacing && !sameCell(p, price)) out.push(p)
    else if (!placed) {
      out.push(price)
      placed = true
    }
  }
  return placed ? out : [...out, price]
}

type GridProps = {
  prices: Price[]
  onChange: (prices: Price[]) => void
  /** A cell a single amount does not hold: edited in the full pricing form. */
  onEditCell: (currency: string, period: BillingPeriod) => void
  defaultCurrency?: string
}

/** An add-on's prices to edit: one per currency and billing period, as a plan charge's. */
export const AddOnPricesGrid = ({ prices, onChange, onEditCell, defaultCurrency }: GridProps) => {
  const [extraCurrencies, setExtraCurrencies] = useState<string[]>([])
  const [extraPeriods, setExtraPeriods] = useState<BillingPeriod[]>([])
  const tenantCurrencies = useQuery(listTenantCurrencies).data?.currencies ?? []
  const first = prices[0]
  const termed = first ? hasTerm(first) : true
  const rows = orderCurrencies(
    [...prices.map(p => p.currency), ...extraCurrencies],
    defaultCurrency ?? first?.currency ?? ''
  ).filter(Boolean)
  const columns: (BillingPeriod | undefined)[] = termed
    ? TERMS.map(t => t.term).filter(
        t => extraPeriods.includes(t) || prices.some(p => hasTerm(p) && p.cadence === t)
      )
    : [undefined]
  const addableCurrencies = tenantCurrencies.filter(c => !rows.includes(c))
  const addablePeriods = termed ? TERMS.filter(t => !columns.includes(t.term)) : []

  return (
    <div className="space-y-2">
      <div className="overflow-x-auto rounded-md border border-border">
        <table className="w-full text-sm">
          <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
            <tr>
              <th scope="col" className="px-3 py-2 font-medium">
                Currency
              </th>
              {columns.map(period => (
                <th key={period ?? 'any'} scope="col" className="px-3 py-2 font-medium">
                  {period === undefined ? 'Price' : `Billed ${termAdverb(period)}`}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {rows.map(currency => (
              <tr key={currency}>
                <th scope="row" className="whitespace-nowrap px-3 py-2 text-left font-normal">
                  <span className="font-mono font-medium">{currency}</span>
                </th>
                {columns.map(period => (
                  <td key={period ?? 'any'} className="px-2 py-1.5">
                    <GridCell
                      prices={prices}
                      currency={currency}
                      period={period ?? first?.cadence ?? TERMS[0].term}
                      onChange={onChange}
                      onEditCell={onEditCell}
                    />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex flex-wrap gap-2">
        {addablePeriods.length > 0 && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="sm" hasIcon>
                <PlusIcon size={12} /> Billing period
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              {addablePeriods.map(t => (
                <DropdownMenuItem
                  key={t.term}
                  onSelect={() => setExtraPeriods(prev => [...prev, t.term])}
                >
                  {t.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
        {addableCurrencies.length > 0 && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="sm" hasIcon>
                <PlusIcon size={12} /> Currency
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="max-h-64 overflow-y-auto">
              {addableCurrencies.map(c => (
                <DropdownMenuItem
                  key={c}
                  className="font-mono"
                  onSelect={() => setExtraCurrencies(prev => [...prev, c])}
                >
                  {c}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
    </div>
  )
}

const GridCell = ({
  prices,
  currency,
  period,
  onChange,
  onEditCell,
}: {
  prices: Price[]
  currency: string
  period: BillingPeriod
  onChange: GridProps['onChange']
  onEditCell: GridProps['onEditCell']
}) => {
  const [draft, setDraft] = useState<string>()
  const own = cellPrices(prices, currency, period)[0]
  const template = own ?? templateFor(prices, currency, period)
  const simple = template ? simpleAmountOf(template) !== undefined : false
  const termed = template ? hasTerm(template) : true

  const open = () => {
    if (!template || !simple) return onEditCell(currency, period)
    setDraft((own && simpleAmountOf(own)) ?? '')
  }

  if (draft !== undefined && template) {
    const valid = isAmount(draft)
    const submit = (event: FormEvent) => {
      event.preventDefault()
      if (!valid) return
      const price = priceOfInput(priceInputOf(template, draft.trim(), period, currency))
      onChange(withPrice(prices, price, own))
      setDraft(undefined)
    }
    return (
      <form
        onSubmit={submit}
        onKeyDown={e => e.key === 'Escape' && setDraft(undefined)}
        className="flex items-center gap-1"
      >
        <div className="flex h-8 min-w-0 flex-1 items-center gap-1 rounded-md border border-input bg-background px-2 focus-within:ring-1 focus-within:ring-ring">
          <span className="font-mono text-[11px] text-muted-foreground">{currency}</span>
          <Input
            autoFocus
            inputMode="decimal"
            aria-label={`${currency}${termed ? `, ${termLabel(period)}` : ''}`}
            aria-invalid={!valid}
            className="h-7 min-w-[5rem] border-0 px-0 text-right font-mono tabular-nums shadow-none focus-visible:ring-0"
            value={draft}
            onChange={e => setDraft(e.target.value)}
          />
        </div>
        <Button type="submit" size="icon" variant="ghost" className="size-7" disabled={!valid} aria-label="Set price">
          <CheckIcon size={13} />
        </Button>
        <Button type="button" size="icon" variant="ghost" className="size-7" onClick={() => setDraft(undefined)} aria-label="Cancel">
          <XIcon size={13} />
        </Button>
      </form>
    )
  }

  const base = 'flex w-full flex-col items-start gap-0.5 rounded-md px-2.5 py-1.5 text-left text-sm'
  if (!own) {
    // A gap: a subscription in this currency on this period is not offered the add-on.
    return (
      <button
        type="button"
        onClick={open}
        className={cn(base, 'border border-dashed border-warning/50 bg-warning/10 text-warning hover:bg-warning/20')}
      >
        + Add price
      </button>
    )
  }

  const text = priceText(own, {})
  return (
    <div className="group/cell relative">
      <button
        type="button"
        onClick={open}
        className={cn(base, 'border border-border bg-background transition-colors hover:bg-accent')}
      >
        <span className="font-semibold tabular-nums">{text.main}</span>
        {text.suffix && <span className="text-xs text-muted-foreground">{text.suffix}</span>}
      </button>
      {prices.length > 1 && (
        <button
          type="button"
          aria-label={`Remove the price in ${currency}`}
          title="Remove this price"
          onClick={() => onChange(prices.filter(p => p !== own))}
          className="absolute right-1 top-1 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/cell:opacity-100"
        >
          <XIcon size={12} />
        </button>
      )}
    </div>
  )
}
