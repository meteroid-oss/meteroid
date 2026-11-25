import { skipToken } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Skeleton,
} from '@md/ui'
import { LockIcon } from 'lucide-react'
import { Fragment, useState } from 'react'
import { useSearchParams } from 'react-router'

import { CurrencyBar } from '@/features/plans/currencies/CurrencyBar'
import {
  optionCell,
  orderCurrencies,
  pricedCurrencies,
  unpricedIn,
} from '@/features/plans/currencies/priceCells'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { useQuery } from '@/lib/connectrpc'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

import { PriceChip } from './ChargeRow'
import { OptionBar } from './PlanOptions'
import { countsWords, rowWords } from './billingWords'
import { contextFrom, firstInvoiceLine } from './planLines'

import type { ContextOf } from './planLines'
import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

/** A published version's prices, one option and currency at a time; "View all" shows the grid. */
export const LockedPrices = () => {
  const { version } = usePlanWithVersion()
  const [params, setParams] = useSearchParams()
  const [viewingAll, setViewingAll] = useState(false)
  const list = useQuery(
    listPriceComponents,
    version ? { planVersionId: version.id } : skipToken
  ).data
  if (!version) return null
  const components = list?.components ?? []
  const options: OptionLike[] = version.options
  const currencies = orderCurrencies(
    [version.currency, ...pricedCurrencies(components)],
    version.currency
  )
  const contextOf = contextFrom(list)
  // Shared with the draft editor through the URL; a currency it is not sold in falls back.
  const picked = params.get('currency')
  const currency = picked && currencies.includes(picked) ? picked : currencies[0]
  const selected = options.find(option => option.alias === params.get('option')) ?? options[0]
  const setQuery = (key: 'currency' | 'option', value: string) =>
    setParams(
      prev => {
        const query = new URLSearchParams(prev)
        query.set(key, value)
        return query
      },
      { replace: true }
    )
  const invoice = selected && firstInvoiceLine(components, selected.id, currency, contextOf)

  return (
    <section
      aria-labelledby="locked-prices"
      className="space-y-3 rounded-lg border border-border bg-card p-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="space-y-1">
          <h2 id="locked-prices" className="flex items-center gap-2 text-sm font-semibold">
            <LockIcon size={13} className="text-muted-foreground" /> Prices · locked in v
            {version.version}
          </h2>
          <p className="text-xs text-muted-foreground">
            Subscriptions on v{version.version} bill these prices. To change one, edit a draft:
            publishing it leaves them on v{version.version} until you migrate them.
          </p>
        </div>
        {components.length > 0 && options.length * currencies.length > 1 && (
          <Button
            variant="outline"
            size="sm"
            className="h-7 shrink-0 text-xs"
            onClick={() => setViewingAll(true)}
          >
            View all
          </Button>
        )}
      </div>
      {!list || !selected ? (
        <Skeleton className="h-24 w-full" />
      ) : components.length === 0 ? (
        <p className="text-sm text-muted-foreground">No charges: nothing is billed.</p>
      ) : (
        <>
          {(options.length > 1 || currencies.length > 1) && (
            <div className="flex flex-wrap items-center justify-between gap-3">
              {options.length > 1 ? (
                <OptionBar
                  options={options}
                  selected={selected}
                  isDraft={false}
                  readOnly
                  warns={option => unpricedIn(components, option.id, currency).length > 0}
                  onSelect={option => setQuery('option', option.alias)}
                />
              ) : (
                <span />
              )}
              {currencies.length > 1 && (
                <CurrencyBar
                  tabs={currencies}
                  currency={currency}
                  defaultCurrency={version.currency}
                  components={components}
                  options={options}
                  isDraft={false}
                  readOnly
                  onCurrency={next => setQuery('currency', next)}
                />
              )}
            </div>
          )}
          <ul className="divide-y divide-border/60 rounded-md border border-border">
            {components.map(component => (
              <LockedCharge
                key={component.id}
                component={component}
                option={selected}
                currency={currency}
                contextOf={contextOf}
              />
            ))}
          </ul>
          {invoice && <p className="text-xs text-muted-foreground">{invoice}</p>}
        </>
      )}
      {viewingAll && (
        <AllPricesDialog
          version={version.version}
          components={components}
          options={options}
          currencies={currencies}
          contextOf={contextOf}
          onClose={() => setViewingAll(false)}
        />
      )}
    </section>
  )
}

/** A charge in the option and currency shown: what it is, and its price chip or "Not sold". */
const LockedCharge = ({
  component,
  option,
  currency,
  contextOf,
}: {
  component: PriceComponent
  option: OptionLike
  currency: string
  contextOf: ContextOf
}) => {
  const ctx = contextOf(component)
  const included = component.prices.length === 0
  const cell = optionCell(component, option.id, currency)
  const words = rowWords(cell, currency, ctx)
  const description = included
    ? 'Included without a price'
    : [countsWords(cell[0] ?? component.prices[0], ctx), words?.billed].filter(Boolean).join(' · ')

  return (
    <li className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5 px-3 py-2.5">
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
        <span className="truncate text-sm font-semibold">{component.name}</span>
        <span className="text-xs text-muted-foreground">{description}</span>
      </div>
      {included ? (
        <span className="text-sm text-muted-foreground">Included</span>
      ) : (
        <PriceChip
          prices={cell}
          currency={currency}
          missing="Not sold"
          ctx={ctx}
          isDraft={false}
          onEdit={() => undefined}
        />
      )}
    </li>
  )
}

/** Every price of the version: its charges × options, currency by currency. */
const AllPricesDialog = ({
  version,
  components,
  options,
  currencies,
  contextOf,
  onClose,
}: {
  version: number
  components: PriceComponent[]
  options: OptionLike[]
  currencies: string[]
  contextOf: ContextOf
  onClose: () => void
}) => (
  <Dialog open onOpenChange={open => !open && onClose()}>
    <DialogContent className="max-w-[min(64rem,calc(100vw-2rem))]">
      <DialogHeader>
        <DialogTitle>Prices · locked in v{version}</DialogTitle>
        <DialogDescription>
          Every charge in each {options.length > 1 ? 'option and currency' : 'currency'}.
        </DialogDescription>
      </DialogHeader>
      <div className="max-h-[70vh] overflow-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-left text-xs text-muted-foreground">
              <th scope="col" className="py-2 pr-4 font-medium">
                Charge
              </th>
              {options.map(option => (
                <th key={option.id} scope="col" className="py-2 pr-4 font-medium">
                  {options.length > 1 ? option.name : 'Price'}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {currencies.map(currency => (
              <Fragment key={currency}>
                {currencies.length > 1 && (
                  <tr>
                    <th
                      scope="rowgroup"
                      colSpan={options.length + 1}
                      className="pb-1 pt-3 text-left font-mono text-xs font-medium text-muted-foreground"
                    >
                      {currency}
                    </th>
                  </tr>
                )}
                {components.map(component => {
                  const ctx = contextOf(component)
                  return (
                    <tr key={component.id} className="border-b border-border/60 last:border-0">
                      <th scope="row" className="py-2 pr-4 text-left font-medium">
                        {component.name}
                      </th>
                      {options.map(option => {
                        const words =
                          component.prices.length === 0
                            ? 'Included'
                            : rowWords(optionCell(component, option.id, currency), currency, ctx)
                                ?.amount
                        return (
                          <td
                            key={option.id}
                            className={
                              words ? 'py-2 pr-4 tabular-nums' : 'py-2 pr-4 text-muted-foreground'
                            }
                          >
                            {words ?? 'Not sold'}
                          </td>
                        )
                      })}
                    </tr>
                  )
                })}
              </Fragment>
            ))}
          </tbody>
        </table>
      </div>
    </DialogContent>
  </Dialog>
)
