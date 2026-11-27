import type { FunctionComponent } from 'react'

/** Not sorted: both directions, dimmed below the header label. */
export const SortableDefaultIndicator: FunctionComponent = () => (
  <svg
    width="6"
    height="10"
    viewBox="0 0 6 10"
    fill="none"
    aria-hidden="true"
    className="text-faint-foreground"
  >
    <path d="M3 0 0 4h6L3 0Z" fill="currentColor" />
    <path d="M3 10 6 6H0l3 4Z" fill="currentColor" />
  </svg>
)

/** Sorted: one filled chevron for the direction, in the label's own colour. */
export const SortedIndicator: FunctionComponent<{ direction: 'asc' | 'desc' }> = ({
  direction,
}) => (
  <svg width="6" height="4" viewBox="0 0 6 4" fill="none" aria-hidden="true">
    <path d={direction === 'asc' ? 'M3 0 6 4H0l3-4Z' : 'M3 4 0 0h6L3 4Z'} fill="currentColor" />
  </svg>
)

export const SortableIndicatorContainer: FunctionComponent<{ children: React.ReactNode }> = ({
  children,
}) => (
  // As wide as the widest indicator, so the label doesn't shift when the direction changes.
  <div className="flex w-1.5 shrink-0 justify-center">{children}</div>
)
