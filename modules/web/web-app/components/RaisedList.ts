/**
 * A list of rows raised a step above its surface, as the row groups in Settings: in dark a fill
 * lighter than the surface (white at 3.5% over it), framed, rounded and divided as the tables.
 * Used for the customer's subscriptions and for entitlements.
 */
export const raisedList =
  'divide-y divide-border overflow-hidden rounded-lg border border-border bg-card dark:bg-foreground/[0.035]'

/** A row in a raised list: 16px in, 12px above and below. */
export const raisedListRow = 'px-4 py-3'

/** A row that leads somewhere: a light tint on hover. */
export const raisedListLinkRow = 'px-4 py-3 transition-colors hover:bg-foreground/[0.04]'
