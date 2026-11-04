import type { BadgeProps } from '@md/ui'

// Mirrors `analytics::lifecycle::LifecycleState` snake_case reprs. Shared by the
// customer badge and the reports `customer_type` breakdown legend/axis.
export type LifecycleState =
  | 'active_paying'
  | 'trialing'
  | 'free'
  | 'past_due'
  | 'churned'
  | 'churned_trial'
  | 'closed_lost'
  | 'lead'
  | 'dormant'
  | 'unknown'

const LIFECYCLE_LABELS: Record<LifecycleState, string> = {
  active_paying: 'Active paying',
  trialing: 'Trialing',
  free: 'Free',
  past_due: 'Past due',
  churned: 'Churned',
  churned_trial: 'Churned trial',
  closed_lost: 'Closed-lost',
  lead: 'Lead',
  dormant: 'Dormant',
  // The `unknown` wire value (a customer with no billing signals yet) reads as "New".
  unknown: 'New',
}

/** High-level buckets over the 10 lifecycle states, used by the customer-list
 * Status filter. Each state belongs to exactly one bucket. */
export interface LifecycleBucket {
  key: string
  label: string
  states: LifecycleState[]
}

export const LIFECYCLE_BUCKETS: LifecycleBucket[] = [
  { key: 'active', label: 'Active', states: ['active_paying', 'trialing', 'free'] },
  { key: 'at_risk', label: 'At risk', states: ['past_due', 'dormant'] },
  { key: 'churned', label: 'Churned', states: ['churned', 'churned_trial', 'closed_lost'] },
  { key: 'prospects', label: 'Prospects', states: ['lead', 'unknown'] },
]

const LIFECYCLE_VARIANTS: Record<LifecycleState, BadgeProps['variant']> = {
  active_paying: 'success',
  trialing: 'default',
  free: 'secondary',
  past_due: 'warning',
  churned: 'destructive',
  churned_trial: 'secondary',
  closed_lost: 'destructive',
  lead: 'outline',
  dormant: 'ghost',
  unknown: 'ghost',
}

/** Selectable lifecycle states (excludes the `unknown` fall-through). Drives the
 * segment-builder customer-type multi-select. */
export const LIFECYCLE_STATES: LifecycleState[] = (
  Object.keys(LIFECYCLE_LABELS) as LifecycleState[]
).filter(s => s !== 'unknown')

/** Human label for a lifecycle snake_case value; falls back to the raw value. */
export const lifecycleStateLabel = (state: string): string =>
  LIFECYCLE_LABELS[state as LifecycleState] ?? state

/** Badge variant for a lifecycle state; defaults to `ghost` for unknown values. */
export const lifecycleStateVariant = (state: string): BadgeProps['variant'] =>
  LIFECYCLE_VARIANTS[state as LifecycleState] ?? 'ghost'
