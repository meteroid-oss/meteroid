import { formatDateOnly, parseDateField } from '@/lib/utils/date'

// Billing dates are calendar days stored at UTC midnight: format them in UTC so they never shift.
const dayMonth = (date: string) =>
  parseDateField(date).toLocaleDateString('en-US', {
    timeZone: 'UTC',
    month: 'short',
    day: 'numeric',
  })

const sameUtcYear = (a: string, b: string) =>
  parseDateField(a).getUTCFullYear() === parseDateField(b).getUTCFullYear()

/** "Oct 29", with the year when it is not the current one. */
export const formatShortDay = (date: string) =>
  sameUtcYear(date, new Date().toISOString()) ? dayMonth(date) : formatDateOnly(date)

/** "Sep 29 – Oct 29, 2026", or both years when they differ. */
export const formatDateRange = (start: string, end: string, separator = '–') =>
  sameUtcYear(start, end)
    ? `${dayMonth(start)} ${separator} ${formatDateOnly(end)}`
    : `${formatDateOnly(start)} ${separator} ${formatDateOnly(end)}`
