import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
  cn,
} from '@md/ui'
import { CheckIcon, PlusIcon, XIcon } from 'lucide-react'
import { FormEvent, useState } from 'react'

import { optionCell, orderCurrencies, sameInEveryOption } from '@/features/plans/currencies/priceCells'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { useQuery } from '@/lib/connectrpc'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import { useChargeContext } from './ChargeRow'
import { chipParts, countsWords, priceWords } from './billingWords'
import { isAmount, priceInputOf, simpleAmountOf, templateFor } from './chargeFormat'

import type { ChargeMutations } from '@/features/plans/charges/useChargeMutations'
import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

type Props = {
  productName: string
  charges: PriceComponent[]
  /** The currencies the plan prices in; others can be added here. */
  currencies: string[]
  options: OptionLike[]
  defaultCurrency: string
  isDraft: boolean
  mutations: ChargeMutations
  /** Opens the charge's own editor on a cell a single amount does not hold. */
  onEditCell: (component: PriceComponent, currency: string, option: OptionLike) => void
  onClose: () => void
}

/** One product's prices in every currency: a row per currency, a column per option. */
export const ProductPricesDialog = ({
  productName,
  charges,
  currencies,
  options,
  defaultCurrency,
  isDraft,
  mutations,
  onEditCell,
  onClose,
}: Props) => {
  const [extraCurrencies, setExtraCurrencies] = useState<string[]>([])
  const tenantCurrencies = useQuery(listTenantCurrencies).data?.currencies ?? []
  const { plan, version } = usePlanWithVersion()
  const rows = orderCurrencies([...currencies, ...extraCurrencies], defaultCurrency)
  const addableCurrencies = tenantCurrencies.filter(c => !rows.includes(c))
  const versionLine =
    plan && version ? `${plan.name}, ${version.isDraft ? 'draft ' : ''}v${version.version}. ` : ''

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-w-[min(64rem,calc(100vw-2rem))]">
        <DialogHeader>
          <DialogTitle>{productName} · all prices</DialogTitle>
          <DialogDescription>
            {versionLine}
            {options.length > 1
              ? 'One price per currency and option: an option without one is not sold in that currency.'
              : 'One price per currency: the plan is not sold in a currency without one.'}
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[70vh] space-y-6 overflow-auto">
          {charges.map(component => (
            <ChargeGrid
              key={component.id}
              component={component}
              rows={rows}
              options={options}
              isDraft={isDraft}
              mutations={mutations}
              onEditCell={onEditCell}
            />
          ))}
        </div>
        <div className="flex flex-wrap gap-2 border-t border-border pt-3">
          {isDraft && addableCurrencies.length > 0 && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="sm" hasIcon className="border-dashed">
                  <PlusIcon size={12} /> Price in another currency
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
          <Button variant="secondary" size="sm" className="ml-auto" onClick={onClose}>
            Done
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

const ChargeGrid = ({
  component,
  rows,
  options,
  isDraft,
  mutations,
  onEditCell,
}: {
  component: PriceComponent
  rows: string[]
  options: OptionLike[]
  isDraft: boolean
  mutations: ChargeMutations
  onEditCell: Props['onEditCell']
}) => {
  const ctx = useChargeContext(component)
  const included = component.prices.length === 0
  const several = options.length > 1

  return (
    <section className="space-y-2">
      <div className="flex flex-wrap items-baseline gap-x-2">
        <h3 className="text-sm font-semibold">{component.name}</h3>
        {!included && (
          <span className="text-xs text-muted-foreground">
            {countsWords(component.prices[0], ctx)}
          </span>
        )}
      </div>
      {included ? (
        <p className="text-sm text-muted-foreground">
          Included in every currency, without a price.
        </p>
      ) : (
        <div className="overflow-x-auto rounded-md border border-border">
          <table className="w-full table-fixed text-sm">
            <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
              <tr>
                <th scope="col" className="w-28 px-3 py-2 font-medium">
                  Currency
                </th>
                {options.map(option => (
                  <th key={option.id} scope="col" className="px-3 py-2 font-medium">
                    {several ? option.name : 'Price'}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y divide-border">
              {rows.map(currency => {
                // One price for every option: one cell across the row.
                const merged = sameInEveryOption(component, options, currency)
                const cells = merged ? [options] : options.map(option => [option])
                return (
                  <tr key={currency}>
                    <th scope="row" className="px-3 py-2 text-left font-normal">
                      <span className="rounded border border-border px-1.5 py-0.5 font-mono text-xs font-medium">
                        {currency}
                      </span>
                    </th>
                    {cells.map(cellOptions => (
                      <td key={cellOptions[0].id} colSpan={cellOptions.length} className="px-2 py-1.5">
                        <GridCell
                          component={component}
                          currency={currency}
                          options={cellOptions}
                          several={several}
                          ctx={ctx}
                          isDraft={isDraft}
                          mutations={mutations}
                          onEditCell={onEditCell}
                        />
                      </td>
                    ))}
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  )
}

const GridCell = ({
  component,
  currency,
  options,
  several,
  ctx,
  isDraft,
  mutations,
  onEditCell,
}: {
  component: PriceComponent
  currency: string
  /** The options the cell prices: one, or all of them when they share it. */
  options: OptionLike[]
  /** The plan has several options: the cell's words name its own. */
  several: boolean
  ctx: ReturnType<typeof useChargeContext>
  isDraft: boolean
  mutations: ChargeMutations
  onEditCell: Props['onEditCell']
}) => {
  const [draft, setDraft] = useState<string>()
  const option = options[0]
  const shared = options.length > 1
  const optionIds = options.map(o => o.id)
  const cell = optionCell(component, option.id, currency)
  const own = cell[0]
  const template = own ?? templateFor(component.prices, currency, undefined)
  const simple = template ? simpleAmountOf(template) !== undefined : false
  const where = several && !shared ? `${option.name} in ${currency}` : `in ${currency}`

  const open = () => {
    if (!template || !simple) return onEditCell(component, currency, option)
    setDraft((own && simpleAmountOf(own)) ?? '')
  }

  if (draft !== undefined && template) {
    const valid = isAmount(draft)
    const submit = (event: FormEvent) => {
      event.preventDefault()
      if (!valid) return
      mutations
        .setCell(component, optionIds, priceInputOf(template, draft.trim(), template.cadence, currency))
        .then(() => setDraft(undefined))
        .catch(() => undefined)
    }
    return (
      <form
        onSubmit={submit}
        onKeyDown={e => e.key === 'Escape' && setDraft(undefined)}
        className="flex items-center gap-1"
      >
        <div className="flex h-11 min-w-0 flex-1 items-center gap-1 rounded-md border border-input bg-background px-2 focus-within:ring-1 focus-within:ring-ring">
          <span className="font-mono text-[11px] text-muted-foreground">{currency}</span>
          <Input
            autoFocus
            inputMode="decimal"
            aria-label={`${component.name}, ${currency}${several && !shared ? `, ${option.name}` : ''}`}
            aria-invalid={!valid}
            className="h-7 min-w-[5rem] border-0 px-0 text-right font-mono tabular-nums shadow-none focus-visible:ring-0"
            value={draft}
            onChange={e => setDraft(e.target.value)}
          />
        </div>
        <Button type="submit" size="icon" variant="ghost" className="size-7" disabled={!valid || mutations.pending} aria-label="Save price">
          <CheckIcon size={13} />
        </Button>
        <Button type="button" size="icon" variant="ghost" className="size-7" onClick={() => setDraft(undefined)} aria-label="Cancel">
          <XIcon size={13} />
        </Button>
      </form>
    )
  }

  const box = 'flex min-h-11 w-full flex-col items-start justify-center gap-0.5 rounded-md border px-3 py-1.5 text-left'
  if (!own) {
    const line = isDraft ? `needed to sell ${where}` : `not sold ${where}`
    return isDraft ? (
      <button
        type="button"
        onClick={open}
        className={cn(box, 'border-dashed border-warning/50 bg-warning/10 hover:bg-warning/20')}
      >
        <span className="text-sm font-medium text-warning">+ Add price</span>
        <span className="text-xs text-warning/80">{line}</span>
      </button>
    ) : (
      <span className={cn(box, 'border-dashed border-border text-muted-foreground')}>
        <span className="text-sm">No price</span>
        <span className="text-xs">{line}</span>
      </span>
    )
  }

  const { strong, quiet } = chipParts(priceWords(own, ctx))
  const levels = cell.length - 1
  const unit = [
    quiet,
    levels > 0 && `+${levels} ${levels === 1 ? 'level' : 'levels'}`,
    shared && 'same in every option',
  ]
    .filter(Boolean)
    .join(' · ')
  const body = (
    <>
      <span className="text-sm font-semibold tabular-nums">{strong}</span>
      {unit && <span className="text-xs text-muted-foreground">{unit}</span>}
    </>
  )
  const style = cn(box, 'border-border bg-background')
  if (!isDraft) return <span className={style}>{body}</span>
  return (
    <div className="group/cell relative">
      <button type="button" onClick={open} className={cn(style, 'transition-colors hover:bg-accent')}>
        {body}
      </button>
      <button
        type="button"
        aria-label={`Remove the price ${where}`}
        title="Remove this price"
        onClick={() =>
          void mutations.clearCell(component, optionIds, currency).catch(() => undefined)
        }
        className="absolute right-1 top-1 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/cell:opacity-100"
      >
        <XIcon size={12} />
      </button>
    </div>
  )
}
