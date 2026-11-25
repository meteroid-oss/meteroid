import { skipToken } from '@connectrpc/connect-query'
import {
  Button,
  Checkbox,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Skeleton,
} from '@md/ui'
import { useAtom } from 'jotai'
import { MoreHorizontalIcon, PlusIcon, Settings2Icon, Trash2Icon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useSearchParams } from 'react-router'

import { ChargeRow } from '@/features/plans/canvas/ChargeRow'
import { OptionBar } from '@/features/plans/canvas/PlanOptions'
import { ProductBlock, groupByProduct } from '@/features/plans/canvas/ProductBlock'
import { ProductPricesDialog } from '@/features/plans/canvas/ProductPricesDialog'
import { contextFrom, firstInvoice } from '@/features/plans/canvas/planLines'
import { useOptionMutations } from '@/features/plans/canvas/useOptionMutations'
import { AddProductDialog } from '@/features/plans/charges/AddProductDialog'
import { InlineChargeEditor } from '@/features/plans/charges/InlineChargeEditor'
import { useChargeMutations } from '@/features/plans/charges/useChargeMutations'
import { CurrencyBar } from '@/features/plans/currencies/CurrencyBar'
import {
  orderCurrencies,
  pricedCurrencies,
  sameInEveryOption,
  unpricedIn,
} from '@/features/plans/currencies/priceCells'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { ComponentRefsProvider } from '@/features/plans/pricecomponents/componentRefs'
import {
  editedComponentsAtom,
  priceCellRequestAtom,
} from '@/features/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { getResolvedEntitlementsForPlanVersion } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

const SIDE_BY_SIDE = 'meteroid.plan-editor.options-side-by-side'

/** "Show all options side by side", remembered in this browser. */
const useSideBySide = () => {
  const [on, setOn] = useState(() => {
    try {
      return window.localStorage.getItem(SIDE_BY_SIDE) === '1'
    } catch {
      return false
    }
  })
  const set = (next: boolean) => {
    setOn(next)
    try {
      window.localStorage.setItem(SIDE_BY_SIDE, next ? '1' : '0')
    } catch {
      // A preference only: the table still switches.
    }
  }
  return [on, set] as const
}

/** The plan's products and their charges, priced per option in one currency at a time. */
export const PriceComponentSection = () => {
  const confirm = useConfirmationModal()
  const { version } = usePlanWithVersion()
  const versionId = version?.id
  const versionCurrency = version?.currency ?? 'USD'
  const options: OptionLike[] = version?.options ?? []
  const isDraft = useIsDraftVersion()
  const [edited, setEdited] = useAtom(editedComponentsAtom)
  const [params, setParams] = useSearchParams()
  const mutations = useChargeMutations()
  const optionMutations = useOptionMutations()

  const componentsQuery = useQuery(
    listPriceComponents,
    versionId ? { planVersionId: versionId } : skipToken
  )
  const list = componentsQuery.data
  const components = list?.components ?? []
  const entitlements = useQuery(
    getResolvedEntitlementsForPlanVersion,
    versionId && env.entitlementsEnabled ? { planVersionId: versionId } : skipToken
  ).data?.entitlements

  const [adding, setAdding] = useState(false)
  // The charge just added, or sent to from the selling gaps banner: scrolled to, lit a moment.
  const [revealed, setRevealed] = useState<string>()
  const [request, setRequest] = useAtom(priceCellRequestAtom)
  const [pricesOf, setPricesOf] = useState<string>()
  // The cell each open editor prices: an option, or the one cell every option shares.
  const [editing, setEditing] = useState<Record<string, { optionId: string; merged: boolean }>>({})

  // Tabs: the version's currency, those priced in, and the one picked to start pricing.
  const picked = params.get('currency') || undefined
  const sold = orderCurrencies([versionCurrency, ...pricedCurrencies(components)], versionCurrency)
  const tabs = orderCurrencies([...sold, ...(picked ? [picked] : [])], versionCurrency)
  const currency = picked ?? tabs[0]
  const contextOf = contextFrom(list)
  const several = options.length > 1
  const invoices = options.map(option => firstInvoice(components, option.id, currency, contextOf))
  // The option the switch shows (and "Edit" opens), kept in the URL by its alias.
  const selected = options.find(option => option.alias === params.get('option')) ?? options[0]
  const [sideBySideSetting, setSideBySide] = useSideBySide()
  const sideBySide = several && sideBySideSetting

  // Every version has an option: none yet is a version still loading.
  const loading = componentsQuery.isPending || options.length === 0
  const featuresOf = (productId: string | undefined) =>
    (entitlements ?? [])
      .filter(e => productId && e.feature?.product?.id === productId)
      .map(e => e.feature?.name ?? '')
      .filter(Boolean)

  // One write per change: two setParams in a row would each start from the same params.
  const setQuery = (change: { currency?: string; option?: string }) =>
    setParams(
      prev => {
        const query = new URLSearchParams(prev)
        if (change.currency) query.set('currency', change.currency)
        if (change.option) query.set('option', change.option)
        return query
      },
      { replace: true }
    )
  const discarding = (go: () => void, pending: boolean) => {
    if (isDraft && pending) confirm(go, { message: 'Discard the pricing edits not saved yet?' })
    else go()
  }
  // An open editor prices the currency it opened on: switching discards it, once confirmed.
  const select = (next: string, force = false, option?: OptionLike) => {
    const go = () => {
      setEdited([])
      setQuery({ currency: next, option: option?.alias })
    }
    if (force) go()
    else discarding(go, edited.length > 0)
  }
  // One option shown: its editors go with it. Side by side, each editor keeps its own option.
  const selectOption = (option: OptionLike) => {
    if (sideBySide) return setQuery({ option: option.alias })
    discarding(() => {
      setEdited([])
      setQuery({ option: option.alias })
    }, edited.length > 0)
  }
  // A new alias for the selected option: the URL follows it, the editors stay.
  const followRename = (renamed: OptionLike) => {
    if (renamed.id === selected?.id) setQuery({ option: renamed.alias })
  }
  const openEditor = (component: PriceComponent, option: OptionLike, merged: boolean) => {
    setEditing(prev => ({ ...prev, [component.id]: { optionId: option.id, merged } }))
    setEdited(prev => (prev.includes(component.id) ? prev : [...prev, component.id]))
  }
  const closeEditor = (component: Pick<PriceComponent, 'id'>) => {
    setEdited(prev => prev.filter(id => id !== component.id))
    setEditing(prev => {
      const next = { ...prev }
      delete next[component.id]
      return next
    })
  }
  // A cell of another currency or option: shown, then opened (open editors are discarded).
  const editCell = (
    component: PriceComponent,
    cellCurrency: string,
    option: OptionLike,
    force = false,
    reveal = false
  ) => {
    const go = () => {
      select(cellCurrency, true, option)
      openEditor(component, option, sameInEveryOption(component, options, cellCurrency))
      if (reveal) setRevealed(component.id)
    }
    if (force) go()
    else discarding(go, edited.length > 0)
  }

  // The banner's chip: its cell opened here, once the charges are in.
  useEffect(() => {
    if (!request || loading || !selected) return
    setRequest(undefined)
    const component = components.find(c => c.id === request.componentId)
    const option = options.find(o => o.id === request.optionId) ?? selected
    if (component) editCell(component, request.currency, option, false, true)
  }, [request, loading, selected])

  // Revealed once rendered (a charge just added arrives with the refetch), the light then fades.
  const present = !!revealed && components.some(c => c.id === revealed)
  useEffect(() => {
    if (!revealed || !present) return
    const frame = requestAnimationFrame(() => {
      const target = document.querySelector(`[data-charge="${CSS.escape(revealed)}"]`)
      if (!target) return
      const tall = target.getBoundingClientRect().height > window.innerHeight * 0.6
      target.scrollIntoView({ behavior: 'smooth', block: tall ? 'start' : 'center' })
    })
    const done = window.setTimeout(() => setRevealed(undefined), 2400)
    return () => {
      cancelAnimationFrame(frame)
      window.clearTimeout(done)
    }
  }, [revealed, present])
  const removeCharge = (component: PriceComponent) =>
    confirm(
      () =>
        void mutations
          .remove(component)
          .then(() => closeEditor(component))
          .catch(() => undefined),
      { message: `Remove ${component.name} from this plan version?` }
    )
  const removeProduct = (name: string, charges: PriceComponent[]) =>
    confirm(
      () =>
        void (async () => {
          for (const charge of charges) {
            await mutations.remove(charge)
            closeEditor(charge)
          }
        })().catch(() => undefined),
      {
        header: `Remove ${name}`,
        message:
          charges.length > 1
            ? `Its ${charges.length} charges are removed from this plan version.`
            : 'Its charge is removed from this plan version.',
      }
    )

  if (!version || !versionId) return null
  const groups = groupByProduct(components)
  const dialogGroup = groups.find(group => group.key === pricesOf)
  const shownName = sideBySide
    ? `${currency}, every option`
    : several && selected
      ? `${currency}, ${selected.name}`
      : currency
  const invoiceLine = sideBySide
    ? options
        .map((option, i) => invoices[i] && `${option.name} ${invoices[i]?.head}`)
        .filter(Boolean)
        .join(' · ')
    : (() => {
        const invoice = invoices[Math.max(0, options.findIndex(o => o.id === selected?.id))]
        return invoice && [invoice.head, invoice.later].filter(Boolean).join(' · ')
      })()

  return (
    <section aria-labelledby="plan-pricing" className="space-y-3 pb-8">
      <div className="flex items-center justify-between gap-3">
        <h2 id="plan-pricing" className="text-lg font-semibold">
          Products &amp; pricing
        </h2>
        {isDraft && (
          <Button variant="outline" size="sm" hasIcon onClick={() => setAdding(true)}>
            <PlusIcon size={14} /> Add a product
          </Button>
        )}
      </div>
      {loading || !selected ? (
        <div className="space-y-4" aria-busy="true" aria-label="Loading pricing">
          <Skeleton className="h-9 w-full" />
          <Skeleton className="h-28 w-full" />
          <Skeleton className="h-28 w-full" />
        </div>
      ) : (
        <>
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border pb-3">
            <OptionBar
              options={options}
              selected={selected}
              isDraft={isDraft}
              mutations={optionMutations}
              warns={option => unpricedIn(components, option.id, currency).length > 0}
              onSelect={selectOption}
              onRenamed={followRename}
            />
            <div className="flex items-center gap-1.5">
              <CurrencyBar
                tabs={tabs}
                currency={currency}
                defaultCurrency={versionCurrency}
                components={components}
                options={options}
                isDraft={isDraft}
                onCurrency={next => select(next)}
              />
              <ViewSettings
                sideBySide={sideBySideSetting}
                several={several}
                onSideBySide={setSideBySide}
              />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            {isDraft ? 'Editing' : 'Showing'}{' '}
            <span className="font-medium text-foreground">{shownName}</span>.{' '}
            {isDraft
              ? `Pick another ${several || tabs.length > 1 ? 'option or currency' : 'currency'} to see and edit its prices.`
              : 'Prices are edited in a draft.'}
          </p>

          <ComponentRefsProvider list={list}>
            <div className="grid gap-3">
              {groups.map(({ key, charges }) => {
                const product = list?.products.find(p => p.id === charges[0].productId)
                const name = product?.name ?? charges[0].name
                return (
                  <ProductBlock
                    key={key}
                    highlighted={charges.some(c => c.id === revealed)}
                    productId={charges[0].productId}
                    fallbackName={charges[0].name}
                    features={featuresOf(charges[0].productId)}
                    actions={
                      <>
                        <Button variant="outline" size="sm" className="h-7 text-xs" onClick={() => setPricesOf(key)}>
                          All prices
                        </Button>
                        {isDraft && (
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <Button
                                variant="ghost"
                                size="icon"
                                className="size-7 text-muted-foreground"
                                aria-label={`${name} actions`}
                              >
                                <MoreHorizontalIcon size={14} />
                              </Button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="end">
                              <DropdownMenuItem
                                onSelect={() => removeProduct(name, charges)}
                                className="text-destructive"
                              >
                                <Trash2Icon size={12} className="mr-2" /> Remove product
                              </DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenu>
                        )}
                      </>
                    }
                  >
                    {charges.map(component => {
                      const open = editing[component.id]
                      const option = options.find(o => o.id === open?.optionId) ?? selected
                      return (
                        <div key={component.id} data-charge={component.id}>
                          {isDraft && option && edited.includes(component.id) ? (
                            <InlineChargeEditor
                              key={`${currency}:${option.id}:${open?.merged ?? false}`}
                              component={component}
                              currency={currency}
                              option={option}
                              options={options}
                              merged={open?.merged ?? sameInEveryOption(component, options, currency)}
                              mutations={mutations}
                              onClose={() => closeEditor(component)}
                            />
                          ) : (
                            <ChargeRow
                              key={currency}
                              component={component}
                              currency={currency}
                              currencies={sold}
                              options={options}
                              selected={selected}
                              sideBySide={sideBySide}
                              isDraft={isDraft}
                              onEdit={(cell, merged) => openEditor(component, cell, merged)}
                              onEditElsewhere={(cellCurrency, cell) => editCell(component, cellCurrency, cell)}
                              onRemove={() => removeCharge(component)}
                            />
                          )}
                        </div>
                      )
                    })}
                  </ProductBlock>
                )
              })}

              {list && components.length === 0 && (
                <div className="rounded-lg border border-dashed border-border px-4 py-8 text-center text-sm text-muted-foreground">
                  No products yet.{' '}
                  {isDraft && (
                    <Button variant="link" className="h-auto p-0" onClick={() => setAdding(true)}>
                      Add the first one
                    </Button>
                  )}
                </div>
              )}
            </div>

            {components.length > 0 && (
              <div className="space-y-1 text-xs text-muted-foreground">
                {isDraft && (
                  <p>
                    Click a price to edit it in the {several ? 'currency and option' : 'currency'}{' '}
                    shown. “All prices” opens every {several ? 'currency and option' : 'currency'} of a
                    product.
                  </p>
                )}
                {invoiceLine && <p>First invoice: {invoiceLine}</p>}
              </div>
            )}

            {isDraft && adding && (
              <AddProductDialog
                currency={currency}
                term={BillingPeriod.MONTHLY}
                everyOption={several}
                mutations={mutations}
                onClose={() => setAdding(false)}
                onAdded={id => {
                  setAdding(false)
                  setRevealed(id)
                }}
              />
            )}

            {dialogGroup && (
              <ProductPricesDialog
                productName={
                  list?.products.find(p => p.id === dialogGroup.charges[0].productId)?.name ??
                  dialogGroup.charges[0].name
                }
                charges={dialogGroup.charges}
                currencies={tabs}
                options={options}
                defaultCurrency={versionCurrency}
                isDraft={isDraft}
                mutations={mutations}
                onEditCell={(component, cellCurrency, option) => {
                  setPricesOf(undefined)
                  editCell(component, cellCurrency, option, true)
                }}
                onClose={() => setPricesOf(undefined)}
              />
            )}
          </ComponentRefsProvider>
        </>
      )}
    </section>
  )
}

/** The view's settings, behind ⚙: every option side by side. */
const ViewSettings = ({
  sideBySide,
  several,
  onSideBySide,
}: {
  sideBySide: boolean
  several: boolean
  onSideBySide: (on: boolean) => void
}) => (
  <Popover>
    <PopoverTrigger asChild>
      <Button variant="ghost" size="icon" className="size-8 text-muted-foreground" aria-label="View settings">
        <Settings2Icon size={14} />
      </Button>
    </PopoverTrigger>
    <PopoverContent align="end" className="w-64 p-3">
      <label className="flex items-start gap-2 text-sm">
        <Checkbox
          className="mt-0.5"
          checked={sideBySide && several}
          disabled={!several}
          onCheckedChange={checked => onSideBySide(checked === true)}
        />
        <span>
          Show all options side by side
          {!several && (
            <span className="block text-xs text-muted-foreground">Once the plan has two options.</span>
          )}
        </span>
      </label>
    </PopoverContent>
  </Popover>
)
