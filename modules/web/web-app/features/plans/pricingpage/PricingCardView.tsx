import {
  Badge,
  Button,
  Slider,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
  getSliderTickPosition,
} from '@md/ui'
import { CheckIcon } from 'lucide-react'
import { ReactNode, useState } from 'react'

import {
  type LevelStop,
  levelReading,
  levelStops,
  signupParams,
} from '@/features/plans/pricingpage/cardLevels'
import {
  type CardLine,
  ctaLabelOf,
  formatAmount,
  priceDisplay,
} from '@/features/plans/pricingpage/cardPrice'
import { PricingCard } from '@/rpc/api/pricingpage/v1/pricingpage_pb'

interface Props {
  card: PricingCard
  currency: string
  /** The option the page shows, passed on to sign-up when the plan sells it. */
  option?: string
  /** Under the highlights, e.g. the button editing its copy. */
  footer?: ReactNode
}

/** One plan as a visitor sees it on the pricing page. */
export const PricingCardView = ({ card, currency, option, footer }: Props) => {
  // A usage-levels charge: the visitor picks a level, the price, limit and overage follow.
  const [picked, setPicked] = useState(0)
  const stops = card.price?.levels ? levelStops(card.price.levels, currency) : []
  const stop = Math.min(picked, Math.max(stops.length - 1, 0))
  const reading = levelReading(card, stop, { currency, option })
  // At "More", a self-serve card's button contacts sales too.
  const contact = card.salesLed || reading?.cta.kind === 'contact'
  const contactUrl =
    reading?.cta.kind === 'contact' ? reading.cta.url : card.salesLed ? card.contactUrl : undefined
  // What sign-up is passed: the plan, currency, option, and the level picked as `capacity`.
  const signup = contact
    ? undefined
    : reading?.cta.kind === 'signup'
      ? reading.cta.params
      : signupParams(card, { currency, option })

  return (
    <article
      className={cn(
        'flex h-full flex-col rounded-xl border bg-card p-5 text-card-foreground',
        card.popular ? 'border-brand ring-1 ring-brand' : 'border-border'
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <h4 className="text-base font-semibold leading-tight">{card.planName}</h4>
        {card.popular && (
          <Badge size="sm" className="rounded-full">
            Most popular
          </Badge>
        )}
      </div>
      <p className="mt-2 min-h-5 text-sm text-muted-foreground">{card.tagline}</p>

      <div className="mt-6 min-h-24">
        <CardPriceBlock
          card={card}
          currency={currency}
          option={option}
          stop={stop}
          onStop={setPicked}
        />
      </div>

      {/* A preview: the button shows the copy, it goes nowhere; a sales-led one shows its link,
          a sign-up the parameters it passes on (`data-signup`). */}
      <Button
        asChild
        variant={card.popular ? 'brand' : 'secondary'}
        className="mt-4 w-full cursor-default"
      >
        <span
          title={contact ? contactUrl || 'No contact link yet' : undefined}
          data-signup={signup?.toString()}
        >
          {card.salesLed || !contact ? ctaLabelOf(card) : 'Contact sales'}
        </span>
      </Button>
      {contact && !contactUrl ? (
        <p className="mt-2 text-center text-xs text-muted-foreground">No contact link yet</p>
      ) : reading?.cta.kind === 'contact' && reading.cta.planName ? (
        <p className="mt-2 text-center text-xs text-muted-foreground">
          On the {reading.cta.planName} plan
        </p>
      ) : !card.salesLed && card.trialDays && card.ctaLabel?.trim() ? (
        <p className="mt-2 text-center text-xs text-muted-foreground">
          {card.trialDays}-day free trial
        </p>
      ) : null}

      <div className="mt-5 space-y-3 border-t border-border pt-4">
        {(card.includesPlanName || card.highlights.length > 0) && (
          <p className="text-xs text-muted-foreground">
            {card.includesPlanName ? `Everything in ${card.includesPlanName}, plus` : 'Includes'}
          </p>
        )}
        {card.highlights.length > 0 ? (
          <ul className="space-y-2 text-sm">
            {card.highlights.map((highlight, index) => (
              <li key={`${index}-${highlight.text}`} className="flex gap-2">
                <CheckIcon size={14} className="mt-0.5 shrink-0 text-success" aria-hidden />
                <span>{highlight.text}</span>
              </li>
            ))}
          </ul>
        ) : (
          footer && <p className="text-xs text-muted-foreground">No highlights yet.</p>
        )}
      </div>

      {footer && <div className="mt-auto pt-5">{footer}</div>}
    </article>
  )
}

/** The levels as stops of a slider, each named under it. */
const LevelSlider = ({
  stops,
  value,
  onChange,
}: {
  stops: LevelStop[]
  value: number
  onChange: (stop: number) => void
}) => (
  <div className="relative px-1">
    <Slider
      min={0}
      max={Math.max(stops.length - 1, 1)}
      step={1}
      value={[value]}
      disabled={stops.length < 2}
      onValueChange={([next]) => onChange(next ?? 0)}
      aria-label="Usage level"
    />
    <div aria-hidden className="absolute inset-x-1 top-full mt-2 h-4">
      {stops.map((stop, index) => (
        <span
          key={`${index}-${stop.label}`}
          className={cn(
            'absolute -translate-x-1/2 whitespace-nowrap text-[11px] tabular-nums',
            index === value ? 'font-medium text-foreground' : 'text-muted-foreground'
          )}
          style={{ left: getSliderTickPosition(index, stops.length) }}
        >
          {stop.label}
        </span>
      ))}
    </div>
  </div>
)

interface BlockProps {
  card: PricingCard
  currency: string
  option?: string
  /** The level shown, of a card offering usage levels (`levelStops`); the lowest when unset. */
  stop?: number
  /** Unset: the levels read at `stop`, with no slider (the ladder's card). */
  onStop?: (stop: number) => void
  /** `sm`: the ladder's card, denser. */
  size?: 'md' | 'sm'
}

/** A card's price as the pricing page reads it: the item leading, what it includes, the others in
 * the card's order and, for usage levels, the level's limit and overage. The ladder's card reads
 * the same. */
export const CardPriceBlock = ({
  card,
  currency,
  option,
  stop = 0,
  onStop,
  size = 'md',
}: BlockProps) => {
  const stops = card.price?.levels ? levelStops(card.price.levels, currency) : []
  const at = Math.min(stop, Math.max(stops.length - 1, 0))
  const reading = levelReading(card, at, { currency, option })
  const price = reading?.price ?? priceDisplay(card, currency)
  const big = size === 'md' ? 'text-4xl' : 'text-3xl'

  return (
    <>
      {price.kind === 'free' && (
        <div className="flex items-baseline gap-1.5">
          <span className={cn(big, 'font-semibold tracking-tight tabular-nums')}>
            {formatAmount('0', currency)}
          </span>
          <span className="text-sm text-muted-foreground">forever</span>
        </div>
      )}
      {price.kind === 'custom' && (
        <div className="text-2xl font-semibold tracking-tight">Custom pricing</div>
      )}
      {price.kind === 'unavailable' && (
        <div className="py-2 text-sm text-muted-foreground">{price.label}</div>
      )}
      {price.kind === 'priced' && (
        <>
          <div className="flex flex-wrap items-baseline gap-x-1.5">
            {price.from && <span className="text-sm text-muted-foreground">from</span>}
            <span className={cn(big, 'font-semibold tracking-tight tabular-nums')}>
              {price.amount}
            </span>
            <span className="text-sm text-muted-foreground">{price.per}</span>
          </div>
          {price.included && (
            <p className="mt-1 text-xs text-muted-foreground">{price.included}</p>
          )}
          {price.billed && <p className="mt-1 text-xs text-muted-foreground">{price.billed}</p>}
          {price.note && <p className="mt-1 text-xs text-muted-foreground">{price.note}</p>}
          {price.extras.length > 0 && <CardLines lines={price.extras} size={size} />}
        </>
      )}
      {reading && (
        <div className="mt-4 space-y-3">
          {onStop && <LevelSlider stops={stops} value={at} onChange={onStop} />}
          <div className={cn('space-y-1 text-xs', onStop && 'pt-5')}>
            <p className="font-medium text-foreground">{reading.limit}</p>
            {reading.overage && <p className="text-muted-foreground">{reading.overage}</p>}
          </div>
        </div>
      )}
    </>
  )
}

// Lines shown under the price before "+N more": a visitor reads a few, the ladder fewer.
const SHOWN = { md: 4, sm: 2 }

/** The card's other items, one a line in its order. Past `SHOWN`, "+N more": on the page a click
 * shows them, on the ladder a tooltip lists them. A single line is never hidden. */
const CardLines = ({ lines, size }: { lines: CardLine[]; size: 'md' | 'sm' }) => {
  const [open, setOpen] = useState(false)
  const cut = open || lines.length <= SHOWN[size] + 1 ? lines.length : SHOWN[size]
  const rest = lines.slice(cut)
  const more = `+${rest.length} more`

  return (
    <ul className={cn('space-y-1', size === 'md' ? 'mt-4 text-sm' : 'mt-2 text-xs')}>
      {lines.slice(0, cut).map(line => (
        <li key={line.text} className="flex gap-1.5">
          <span className="text-muted-foreground">+</span>
          <span className="min-w-0">
            {line.figure && (
              <span className="font-medium tabular-nums text-foreground">{line.figure}</span>
            )}
            {line.label && (
              <span className="text-muted-foreground">
                {line.figure && ' '}
                {line.label}
              </span>
            )}
            {line.included && (
              <span className="text-muted-foreground"> · {line.included}</span>
            )}
          </span>
        </li>
      ))}
      {rest.length > 0 && (
        <li>
          {size === 'md' ? (
            <button
              type="button"
              onClick={() => setOpen(true)}
              className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            >
              {more}
            </button>
          ) : (
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  className="text-muted-foreground underline decoration-dotted underline-offset-2 hover:text-foreground"
                >
                  {more}
                </button>
              </TooltipTrigger>
              <TooltipContent multiline className="max-w-72 space-y-0.5 text-xs">
                {rest.map(line => (
                  <p key={line.text}>{line.text}</p>
                ))}
              </TooltipContent>
            </Tooltip>
          )}
        </li>
      )}
    </ul>
  )
}
