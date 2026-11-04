import { HoverCard, HoverCardContent, HoverCardTrigger } from '@md/ui'
import { type ReactNode } from 'react'

import { formatCurrency } from '@/lib/utils/numbers'
import { MinimumCommitment } from '@/rpc/api/plans/v1/models_pb'

/** A selectable scope entry. `id` is a PRODUCT id (the minimum scope is keyed by product). */
export interface MinimumComponentOption {
  id: string
  name: string
  /**
   * True for an inline new-product extra that has no product id yet (the server mints it at
   * create time). It can't be scoped individually until the product exists, so it is shown
   * disabled — it still counts under "All components".
   */
  pending?: boolean
}

/**
 * Derive the product scope options from a list of components (plan price-components, quote
 * components, …). Each component resolves to its product; duplicates (two components sharing a
 * product) are collapsed, and components without a product (legacy) are skipped.
 */
export const productScopeOptions = (
  components: { name: string; productId?: string }[]
): MinimumComponentOption[] => {
  const seen = new Set<string>()
  const out: MinimumComponentOption[] = []
  for (const c of components) {
    if (c.productId && !seen.has(c.productId)) {
      seen.add(c.productId)
      out.push({ id: c.productId, name: c.name })
    }
  }
  return out
}

/**
 * Like {@link productScopeOptions}, but also surfaces inline new-product extras (no product id
 * yet) as `pending` entries instead of silently dropping them. Used by the subscription/quote
 * create wizards: with `allowPending` the entry is selectable and the server resolves it by name
 * at create/conversion time; without it, the entry is shown disabled.
 */
export const scopeOptionsWithPending = (
  components: { name: string; productId?: string }[]
): MinimumComponentOption[] => {
  const seen = new Set<string>()
  const out: MinimumComponentOption[] = []
  components.forEach((c, i) => {
    if (c.productId) {
      if (!seen.has(c.productId)) {
        seen.add(c.productId)
        out.push({ id: c.productId, name: c.name })
      }
    } else {
      // No real id yet — synthesize a stable key so React can render it; it can't be selected.
      out.push({ id: `pending:${i}:${c.name}`, name: c.name, pending: true })
    }
  })
  return out
}

const scopeKind = (commitment: MinimumCommitment) => commitment.scope?.kind?.case

export const scopedProductIds = (commitment?: MinimumCommitment): string[] =>
  commitment?.scope?.kind?.case === 'products' ? commitment.scope.kind.value.productIds : []

/** Names of in-scope inline new-product extras not yet resolved to product ids (quote drafts). */
export const scopedPendingNames = (commitment?: MinimumCommitment): string[] =>
  commitment?.scope?.kind?.case === 'products'
    ? commitment.scope.kind.value.pendingExtraProductNames
    : []

/** "500 EUR" — the formatted amount in major units. */
const formatAmount = (commitment: MinimumCommitment, currency: string): string =>
  formatCurrency(BigInt(commitment.amountSubunit.toString()), currency)

/** Resolve the scoped product names (for the hover card / listing). */
const scopedProductNames = (
  commitment: MinimumCommitment,
  components: MinimumComponentOption[]
): string[] =>
  scopedProductIds(commitment)
    .map(id => components.find(c => c.id === id)?.name)
    .filter((n): n is string => !!n)

/**
 * The scope half of a term value: "All components" inline, or a compact "N components"
 * trigger that reveals the full list in a hover card. Matches the surrounding muted text style.
 */
const ScopeSummary = ({
  commitment,
  components,
}: {
  commitment: MinimumCommitment
  components: MinimumComponentOption[]
}) => {
  if (scopeKind(commitment) === 'allComponents') {
    return <span className="text-muted-foreground">all components</span>
  }

  // Resolved products (by id) + any unresolved pending extras (by name, on quote drafts).
  const names = [...scopedProductNames(commitment, components), ...scopedPendingNames(commitment)]
  const count = scopedProductIds(commitment).length + scopedPendingNames(commitment).length
  const countLabel = `${count} product${count === 1 ? '' : 's'}`

  // Names not loaded yet (products query pending) — show the count without a hover card.
  if (names.length === 0) {
    return <span className="text-muted-foreground">{countLabel}</span>
  }

  return (
    <HoverCard openDelay={80} closeDelay={40}>
      <HoverCardTrigger asChild>
        <span className="text-muted-foreground underline decoration-dotted underline-offset-2 cursor-default">
          {countLabel}
        </span>
      </HoverCardTrigger>
      <HoverCardContent align="start" className="w-auto max-w-xs p-3 text-[13px]">
        <div className="font-medium mb-1.5">Counts toward the floor</div>
        <ul className="space-y-0.5 text-muted-foreground">
          {names.map((name, i) => (
            <li key={i}>{name}</li>
          ))}
        </ul>
      </HoverCardContent>
    </HoverCard>
  )
}

/**
 * Compact, read-only two-line term display. Line 1: label + optional badge. Line 2: the
 * value (`amount per billing period · scope`). Sized at text-[13px] to match the detail rows it sits
 * alongside (quote/subscription panels, signed document).
 */
export const MinimumCommitmentDisplay = ({
  commitment,
  currency,
  components,
  label = 'Minimum commitment',
  badge,
  emptyText = 'No minimum commitment',
  action,
  className = '',
}: {
  commitment?: MinimumCommitment
  currency: string
  components: MinimumComponentOption[]
  label?: ReactNode
  badge?: ReactNode
  emptyText?: string
  /** Optional trailing control (e.g. an edit button), right-aligned. */
  action?: ReactNode
  className?: string
}) => (
  <div className={`flex items-start justify-between gap-3 text-[13px] ${className}`}>
    <div className="min-w-0 space-y-0.5">
      <div className="flex items-center gap-2">
        <span className="text-muted-foreground">{label}</span>
        {commitment && badge}
      </div>
      {commitment ? (
        <div className="flex items-baseline gap-1.5 flex-wrap">
          <span className="font-medium text-foreground">{formatAmount(commitment, currency)}</span>
          <span className="text-muted-foreground">per billing period</span>
          <span className="text-muted-foreground">·</span>
          <ScopeSummary commitment={commitment} components={components} />
        </div>
      ) : (
        <div className="text-muted-foreground">{emptyText}</div>
      )}
    </div>
    {action && <div className="shrink-0">{action}</div>}
  </div>
)

