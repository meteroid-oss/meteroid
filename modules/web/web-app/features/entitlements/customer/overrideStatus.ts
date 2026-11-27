import { parseUtc } from '@/utils/date'

export const ENDS_SOON_DAYS = 7

export type OverrideStatus = 'upcoming' | 'active' | 'endsSoon' | 'ended'

/** Where an override's window stands at `now`: ending soon within a week of its end. */
export function overrideStatus(
  row: { validFrom?: string; validUntil?: string },
  now: number
): OverrideStatus {
  if (row.validFrom && parseUtc(row.validFrom).getTime() > now) return 'upcoming'
  if (!row.validUntil) return 'active'
  const until = parseUtc(row.validUntil).getTime()
  if (until <= now) return 'ended'
  return until - now <= ENDS_SOON_DAYS * 24 * 60 * 60 * 1000 ? 'endsSoon' : 'active'
}
