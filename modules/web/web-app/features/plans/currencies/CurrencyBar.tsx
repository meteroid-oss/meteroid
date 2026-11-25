import { Button, Popover, PopoverContent, PopoverTrigger, cn } from '@md/ui'
import { PlusIcon } from 'lucide-react'
import { useState } from 'react'
import { Link } from 'react-router'

import { EditedInDraft, addButton } from '@/features/plans/canvas/PlanOptions'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import { tabWarns } from './priceCells'

import type { OptionLike } from './priceCells'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

/** The currencies the plan is priced in, as a switch, then "+"; a dot where an option lacks a price. */
export const CurrencyBar = ({
  tabs,
  currency,
  defaultCurrency,
  components,
  options,
  isDraft,
  readOnly,
  onCurrency,
}: {
  tabs: string[]
  currency: string
  defaultCurrency: string
  components: PriceComponent[]
  options: OptionLike[]
  isDraft: boolean
  /** The switch alone, without "+". */
  readOnly?: boolean
  onCurrency: (currency: string) => void
}) => (
  <div className="flex items-center gap-1.5">
    <div
      role="tablist"
      aria-label="Currency"
      className="inline-flex h-8 items-center rounded-md border border-border p-0.5"
    >
      {tabs.map(tab => {
        const selected = tab === currency
        return (
          <button
            key={tab}
            type="button"
            role="tab"
            aria-selected={selected}
            title={tab === defaultCurrency ? `${tab}, the plan's currency` : undefined}
            onClick={() => !selected && onCurrency(tab)}
            className={cn(
              'inline-flex h-full items-center gap-1.5 rounded px-2.5 font-mono text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
              selected ? 'bg-accent text-foreground' : 'text-muted-foreground hover:text-foreground'
            )}
          >
            {tab}
            {tabWarns(components, options, tab) && (
              <span
                role="img"
                aria-label={`A charge has no price in ${tab}`}
                className="size-1.5 rounded-full bg-warning"
              />
            )}
          </button>
        )
      })}
    </div>
    {readOnly ? null : isDraft ? (
      <AddCurrency tabs={tabs} onCurrency={onCurrency} />
    ) : (
      <EditedInDraft>
        <Button variant="outline" size="icon" className={addButton} disabled aria-label="Price in another currency">
          <PlusIcon size={14} />
        </Button>
      </EditedInDraft>
    )}
  </div>
)

/** Picks another of the account's currencies: its tab opens empty, each charge to price. */
const AddCurrency = ({
  tabs,
  onCurrency,
}: {
  tabs: string[]
  onCurrency: (currency: string) => void
}) => {
  const [open, setOpen] = useState(false)
  const basePath = useBasePath()
  const tenantCurrencies = useQuery(listTenantCurrencies).data?.currencies ?? []
  const addable = tenantCurrencies.filter(c => !tabs.includes(c))
  const choose = (currency: string) => {
    setOpen(false)
    onCurrency(currency)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="icon" className={addButton} aria-label="Price in another currency">
          <PlusIcon size={14} />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-56 p-1.5">
        {addable.length > 0 ? (
          <ul aria-label="Currencies" className="flex max-h-64 flex-col overflow-y-auto">
            {addable.map(c => (
              <li key={c}>
                <button
                  type="button"
                  onClick={() => choose(c)}
                  className="w-full rounded px-2 py-1.5 text-left font-mono text-sm hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
                >
                  {c}
                </button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="px-2 py-1.5 text-xs text-muted-foreground">
            The plan is priced in every currency of your account.
          </p>
        )}
        <div className="mt-1 border-t border-border px-2 pt-1.5 pb-0.5">
          <Link
            to={`${basePath}/settings/general`}
            className="text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
          >
            Manage currencies
          </Link>
        </div>
      </PopoverContent>
    </Popover>
  )
}
