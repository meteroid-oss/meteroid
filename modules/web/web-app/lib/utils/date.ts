/** Naive UTC datetimes from the API ("2026-10-31T00:00:00") read as UTC. */
export const parseUtc = (value: string) =>
  new Date(/[zZ]|[+-]\d\d:?\d\d$/.test(value) ? value : `${value}Z`)

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/

/** A date field at its UTC midnight: a yyyy-MM-dd date, or a naive timestamp, which is UTC. */
export const parseDateField = (value: string) =>
  DATE_ONLY.test(value) ? new Date(value) : parseUtc(value)

/** A timestamp in the viewer's day; a yyyy-MM-dd date as that calendar day. */
export const parseAndFormatDate = (dateString: string) => {
  if (!dateString) return 'N/A'
  if (DATE_ONLY.test(dateString)) return formatDateOnly(dateString)
  return new Date(dateString).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

export const parseAndFormatDateOptional = (date?: string) => {
  return date ? parseAndFormatDate(date) : '-'
}

/// Formats a date-anchored value (a yyyy-MM-dd date, or a UTC-midnight timestamp that stands for a
/// date) by its UTC calendar day — so a "2026-08-25" parsed as UTC midnight never renders as the
/// previous day west of UTC. Use for date fields (invoice/billing-period/due dates), NOT for
/// wall-clock timestamps where the viewer's local day is what's wanted.
export const formatDateOnly = (dateString: string) => {
  if (!dateString) return 'N/A'
  return parseDateField(dateString).toLocaleDateString('en-US', {
    timeZone: 'UTC',
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

export const formatDateOnlyOptional = (date?: string) => {
  return date ? formatDateOnly(date) : '-'
}

export const parseAndFormatDateTime = (dateString: string, precision: 'min' | 'sec' = 'min') => {
  if (!dateString) return 'N/A'
  return new Date(dateString).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    minute: 'numeric',
    hour: 'numeric',
    hour12: false,
    second: precision === 'sec' ? 'numeric' : undefined,
  })
}

export const parseDate = (dateString: string): Date => {
  return new Date(dateString)
}
