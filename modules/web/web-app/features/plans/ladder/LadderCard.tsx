import { useSortable } from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import {
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from '@md/ui'
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  GlobeIcon,
  GripVerticalIcon,
  HelpCircleIcon,
  MoreHorizontalIcon,
  PencilIcon,
  TicketPercentIcon,
  XIcon,
} from 'lucide-react'
import { ReactNode } from 'react'
import { Link } from 'react-router'

import { orderCurrencies, termAdverb, termPer } from '@/features/plans/currencies/priceCells'
import { CardPriceBlock } from '@/features/plans/pricingpage/PricingCardView'
import { formatAmount } from '@/features/plans/pricingpage/cardPrice'
import { usePlanCoupons } from '@/features/plans/usePlanCoupons'
import { couponDiscountLabel } from '@/features/productCatalog/coupons/discountLabel'
import { PlanOverview, PlanStatus, PricedOption } from '@/rpc/api/plans/v1/models_pb'
import { PricingCard } from '@/rpc/api/pricingpage/v1/pricingpage_pb'

interface Props {
  plan: PlanOverview
  /** Its place on the ladder, from 1. */
  position: number
  /** Its pricing page card, for an active plan. */
  card?: PricingCard
  currency?: string
  /** The option shown: its alias and name. */
  option?: { alias: string; name: string }
  /** The tenant's currencies: those the plan is not priced in show as gaps. */
  tenantCurrencies: string[]
  reportingCurrency?: string
  selected: boolean
  onSelect: () => void
  /** Unset when the ladder cannot be changed. */
  onMove?: (step: -1 | 1) => void
  first: boolean
  last: boolean
  onEdit?: () => void
  onToggleSalesLed?: () => void
  onRemove?: () => void
}

/** A plan on its product line's ladder: its price as shown, where it sells and what it earns. */
export const LadderCard = ({
  plan,
  position,
  card,
  currency,
  option,
  tenantCurrencies,
  reportingCurrency,
  selected,
  onSelect,
  onMove,
  first,
  last,
  onEdit,
  onToggleSalesLed,
  onRemove,
}: Props) => {
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } =
    useSortable({ id: plan.id, disabled: !onMove })

  const coupons = usePlanCoupons(plan.id, 5)
  const moreCoupons = coupons.total - (coupons.featured ? 1 : 0)

  const sold = orderCurrencies(plan.pricedOptions.map(p => p.currency), currency ?? '')
  const unpriced = tenantCurrencies.filter(code => !sold.includes(code))

  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Translate.toString(transform), transition }}
      onClick={onSelect}
      className={cn(
        'flex min-w-0 flex-col rounded-xl border bg-card p-4 transition-colors',
        selected ? 'border-brand ring-1 ring-brand' : 'border-border hover:border-brand/60',
        isDragging && 'relative z-10 opacity-90 shadow-lg'
      )}
    >
      <div className="flex flex-wrap items-center gap-2">
        {onMove && (
          <button
            type="button"
            ref={setActivatorNodeRef}
            {...attributes}
            {...listeners}
            aria-label={`Move ${plan.name}`}
            className="-ml-1 cursor-grab touch-none rounded p-0.5 text-muted-foreground hover:text-foreground active:cursor-grabbing"
          >
            <GripVerticalIcon size={14} />
          </button>
        )}
        <span className="inline-flex h-6 min-w-6 items-center justify-center rounded-md border border-border px-1.5 text-xs tabular-nums">
          {position}
        </span>
        {card?.popular && <Tag className="bg-brand/15 text-brand">Popular</Tag>}
        {plan.hasDraftVersion && (
          <Tag className="bg-warning/15 text-warning">
            Draft v{(plan.activeVersion?.version ?? 0) + 1}
          </Tag>
        )}
        {plan.planStatus === PlanStatus.INACTIVE && (
          <Tag className="bg-muted text-muted-foreground">Inactive</Tag>
        )}
        {plan.salesLed && <Tag className="bg-muted text-muted-foreground">Sales-led</Tag>}
      </div>

      <h3 className="mt-4 truncate text-base font-semibold leading-tight">{plan.name}</h3>
      <p className="mt-1 line-clamp-2 min-h-5 text-sm text-muted-foreground">
        {plan.description}
      </p>

      <div className="mt-4 min-h-16">
        <Price plan={plan} card={card} currency={currency} option={option} />
      </div>

      <div className="mt-auto pt-4">
        {/* At the foot, the currencies last, so the cards of a ladder line up. */}
        {(sold.length > 0 || coupons.featured || moreCoupons > 0) && (
          <div className="mb-3 space-y-1.5 text-xs text-muted-foreground">
            {(coupons.featured || moreCoupons > 0) && (
              <p className="flex min-w-0 items-center gap-1.5">
                <TicketPercentIcon size={13} className="shrink-0" aria-label="Coupons" />
                {coupons.featured && (
                  // A long code gives way to its discount.
                  <span
                    className="flex min-w-0 gap-1"
                    title={coupons.featured.description || undefined}
                  >
                    <span className="truncate font-mono text-foreground">
                      {coupons.featured.code}
                    </span>
                    <span className="shrink-0">
                      {couponDiscountLabel(coupons.featured.discount)}
                    </span>
                  </span>
                )}
                {moreCoupons > 0 && (
                  <Link
                    to={`${plan.localId}?more=coupons`}
                    onClick={event => event.stopPropagation()}
                    className="shrink-0 underline-offset-2 hover:text-foreground hover:underline"
                  >
                    {coupons.featured
                      ? `+${moreCoupons} more`
                      : `${moreCoupons} ${moreCoupons === 1 ? 'coupon' : 'coupons'}`}
                  </Link>
                )}
              </p>
            )}
            {sold.length > 0 && <Currencies sold={sold} unpriced={unpriced} />}
          </div>
        )}
        <div className="flex items-center justify-between gap-2 border-t border-border pt-3 text-xs text-muted-foreground">
          <span className="tabular-nums">
            {plan.subscriptionCount}{' '}
            {plan.subscriptionCount === 1 ? 'subscription' : 'subscriptions'}
          </span>
          {plan.mrr !== undefined && reportingCurrency && (
            <span className="tabular-nums">{compactMoney(plan.mrr, reportingCurrency)} MRR</span>
          )}
        </div>

        <div className="mt-3 flex items-center gap-1" onClick={event => event.stopPropagation()}>
          <Button variant="outline" size="sm" className="mt-0 h-8 flex-1" asChild>
            <Link to={plan.localId}>Open</Link>
          </Button>
          {onMove && (
            <>
              <MoveButton label="Move left" disabled={first} onClick={() => onMove(-1)}>
                <ArrowLeftIcon size={14} />
              </MoveButton>
              <MoveButton label="Move right" disabled={last} onClick={() => onMove(1)}>
                <ArrowRightIcon size={14} />
              </MoveButton>
            </>
          )}
          {(onEdit || onToggleSalesLed || onRemove) && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="mt-0 size-7 text-muted-foreground"
                  aria-label={`${plan.name} actions`}
                >
                  <MoreHorizontalIcon size={14} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {onEdit && (
                  <DropdownMenuItem onClick={onEdit}>
                    <PencilIcon size={16} className="mr-2" />
                    Edit card
                  </DropdownMenuItem>
                )}
                {onToggleSalesLed && (
                  <DropdownMenuCheckboxItem
                    checked={plan.salesLed}
                    onCheckedChange={onToggleSalesLed}
                  >
                    Sell by contacting sales
                  </DropdownMenuCheckboxItem>
                )}
                {onRemove && (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem onClick={onRemove}>
                      <XIcon size={16} className="mr-2" />
                      Remove from the ladder
                    </DropdownMenuItem>
                  </>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>
      </div>
    </div>
  )
}

/** A small arrow moving the plan along the ladder. */
const MoveButton = ({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string
  disabled: boolean
  onClick: () => void
  children: ReactNode
}) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <Button
        variant="ghost"
        size="icon"
        className="mt-0 size-7 text-muted-foreground"
        disabled={disabled}
        aria-label={label}
        onClick={onClick}
      >
        {children}
      </Button>
    </TooltipTrigger>
    <TooltipContent>{label}</TooltipContent>
  </Tooltip>
)

/** Where the plan sells: its currencies, the one shown first, and the tenant's it is not priced
 * in, the reason on hover. */
const Currencies = ({ sold, unpriced }: { sold: string[]; unpriced: string[] }) => (
  <p className="flex min-w-0 items-center gap-1.5">
    <GlobeIcon size={13} className="shrink-0" aria-label="Currencies" />
    <span className="truncate">
      <span className="font-mono text-foreground">{sold.join(' ')}</span>
      {unpriced.length > 0 && (
        <>
          {' · not in '}
          <span className="font-mono">{unpriced.join(' ')}</span>
        </>
      )}
    </span>
    {unpriced.length > 0 && (
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="shrink-0 hover:text-foreground"
            aria-label="Why it is not sold everywhere"
          >
            <HelpCircleIcon size={13} />
          </button>
        </TooltipTrigger>
        <TooltipContent className="max-w-64 text-xs font-normal">
          Not priced in {listed(unpriced)}: the plan can&rsquo;t be sold in{' '}
          {unpriced.length === 1 ? 'it' : 'them'}, and the pricing page leaves it out there.
        </TooltipContent>
      </Tooltip>
    )}
  </p>
)

/** `EUR`, `EUR or BRL`, `EUR, BRL or GBP`. */
const listed = (codes: string[]) =>
  codes.length < 2 ? codes.join('') : `${codes.slice(0, -1).join(', ')} or ${codes.at(-1)}`

const Tag = ({ className, children }: { className: string; children: ReactNode }) => (
  <span className={cn('rounded-full px-2 py-0.5 text-[11px] font-medium', className)}>
    {children}
  </span>
)

/** The price in `currency` of `option` (by alias), else of the plan's default there (its first
 * option sold and shown) with a note, as the pricing page picks it. */
export const pricedIn = (
  plan: PlanOverview,
  currency: string | undefined,
  alias: string | undefined
): { priced: PricedOption; note?: string } | undefined => {
  const sold = [...plan.options]
    .filter(o => !o.hidden)
    .sort((a, b) => a.position - b.position)
    .flatMap(option => {
      const priced = plan.pricedOptions.find(
        p => p.currency === currency && p.optionId === option.id
      )
      return priced ? [{ option, priced }] : []
    })
  const named =
    alias !== undefined
      ? sold.find(s => s.option.alias.toLowerCase() === alias.toLowerCase())
      : undefined
  if (named) return { priced: named.priced }
  const [first] = sold
  if (!first) return undefined
  if (alias === undefined) return { priced: first.priced }
  const period = first.priced.period
  const note =
    sold.length > 1
      ? `${first.option.name} only`
      : period !== undefined
        ? `Billed ${termAdverb(period)}`
        : undefined
  return { priced: first.priced, note }
}

const Price = ({
  plan,
  card,
  currency,
  option,
}: Pick<Props, 'plan' | 'card' | 'currency' | 'option'>) => {
  // On the pricing page: its card's reading, the same server-side one, in the page's currency
  // and option.
  if (card && currency) {
    return <CardPriceBlock card={card} currency={currency} option={option?.alias} size="sm" />
  }
  if (plan.salesLed && plan.hidePrice) {
    return <div className="text-2xl font-semibold tracking-tight">Custom pricing</div>
  }
  // Off the page: the plans list's headline.
  const shown = currency ? pricedIn(plan, currency, option?.alias) : undefined
  if (!currency || !shown) {
    return (
      <div className="py-2 text-sm text-muted-foreground">
        {currency ? `Not sold in ${currency}` : 'No price yet'}
      </div>
    )
  }
  const { priced, note } = shown
  if (priced.amount === undefined) {
    return <div className="text-2xl font-semibold tracking-tight">Usage-based</div>
  }
  const free = Number(priced.amount) === 0 && !priced.hasUsage && !priced.unit
  const period = priced.period !== undefined && `/ ${termPer(priced.period)}`
  const unit = priced.unit && `/ ${priced.unit}`
  const per = free ? 'forever' : [unit, period].filter(Boolean).join(' ')
  return (
    <>
      <div className="flex flex-wrap items-baseline gap-x-1.5">
        {priced.fromLowest && <span className="text-sm text-muted-foreground">from</span>}
        <span className="text-3xl font-semibold tracking-tight tabular-nums">
          {formatAmount(priced.amount, currency)}
        </span>
        {per && <span className="text-sm text-muted-foreground">{per}</span>}
      </div>
      {note && <p className="mt-1 text-xs text-muted-foreground">{note}</p>}
      {priced.hasUsage && <p className="mt-1 text-xs text-muted-foreground">+ usage</p>}
    </>
  )
}

/** `$13.1K`, `$0`. */
export const compactMoney = (amount: string, currency: string) => {
  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency,
      currencyDisplay: 'narrowSymbol',
      notation: 'compact',
      maximumFractionDigits: 1,
    }).format(Number(amount))
  } catch {
    return `${amount} ${currency}`
  }
}
