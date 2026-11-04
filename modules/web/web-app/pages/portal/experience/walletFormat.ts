import { timestampDate } from '@bufbuild/protobuf/wkt'
import { Code, ConnectError } from '@connectrpc/connect'

import { rollsOver } from '@/features/credit/format'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrency } from '@/lib/utils/numbers'
import { CreditGrantKind, CreditSource } from '@/rpc/portal/wallets/v1/models_pb'

import { date } from './format'

import type {
  CreditGrantView,
  CreditGroup,
  CreditUnitLabel,
  Wallet,
} from '@/rpc/portal/wallets/v1/models_pb'
import type { Timestamp } from '@bufbuild/protobuf/wkt'
import type { CSSProperties } from 'react'

// Transport and server failures carry no wording meant for the customer.
const GENERIC_FAILURES = new Set([
  Code.Internal,
  Code.Unknown,
  Code.Unavailable,
  Code.DeadlineExceeded,
  Code.Canceled,
])

/** A refusal as the server words it for the customer, never with the transport's "[code]" prefix.
 * Internal failures, and errors that never reached the server, get `fallback` instead. */
export const customerError = (cause: unknown, fallback: string) => {
  const error = ConnectError.from(cause)
  return GENERIC_FAILURES.has(error.code) || !error.rawMessage ? fallback : error.rawMessage
}

export const day = (timestamp?: Timestamp) =>
  timestamp ? date(timestampDate(timestamp).toISOString()) : ''

type Unit = Pick<CreditUnitLabel, 'precision' | 'name' | 'pluralName'>

/** A credit-unit amount as a bare count, e.g. "1,720". */
export const creditCount = (minor: bigint, unit: Pick<CreditUnitLabel, 'precision'>) =>
  new Intl.NumberFormat('en-US', {
    minimumFractionDigits: 0,
    maximumFractionDigits: unit.precision,
  }).format(Number(minor) / 10 ** unit.precision)

/** "Halyard credits": the unit's plural name, its casing kept (it may carry a brand). */
export const creditsLabel = (unit: Pick<CreditUnitLabel, 'pluralName'>) => unit.pluralName

/** "Halyard credit" for exactly one, the plural name otherwise. */
export const unitNameFor = (minor: bigint, unit: Unit) =>
  minor === 10n ** BigInt(unit.precision) ? unit.name : unit.pluralName

/** "$975.00" for money, "5,000 Halyard credits" for a credit unit. */
export const walletAmount = (minor: bigint, currency: string, unit?: Unit) =>
  unit ? `${creditCount(minor, unit)} ${unitNameFor(minor, unit)}` : formatCurrency(minor, currency)

const roundToWhole = (minor: bigint, precision: number) => {
  const scale = 10n ** BigInt(precision)
  const half = scale / 2n
  return ((minor >= 0n ? minor + half : minor - half) / scale) * scale
}

/** An estimate, rounded to whole units: "$460" or "5,090". */
export const approxAmount = (minor: bigint, currency: string, unit?: Unit) =>
  unit
    ? creditCount(roundToWhole(minor, unit.precision), unit)
    : formatCurrency(roundToWhole(minor, currencyPrecision(currency)), currency, {
        trimWholeAmounts: true,
      })

const SHORT_UTC = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  timeZone: 'UTC',
})
const FULL_UTC = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
  timeZone: 'UTC',
})
const MONTH_UTC = new Intl.DateTimeFormat('en-US', { month: 'long', timeZone: 'UTC' })

// Period, window and expiry boundaries are calendar dates at UTC midnight: print them in UTC so
// they match the invoice and subscription dates instead of drifting a day in western time zones.
const utcDate = (value: Timestamp | string) =>
  typeof value === 'string' ? new Date(`${value}T00:00:00Z`) : timestampDate(value)

/** "Oct 15" within the current year, "Oct 15, 2027" otherwise. */
export const boundaryDay = (value?: Timestamp | string) => {
  if (!value) return ''
  const d = utcDate(value)
  return d.getUTCFullYear() === new Date().getUTCFullYear()
    ? SHORT_UTC.format(d)
    : FULL_UTC.format(d)
}

/** "Oct 15, 2026". */
export const boundaryDate = (value?: Timestamp | string) =>
  value ? FULL_UTC.format(utcDate(value)) : ''

/** "October". */
export const boundaryMonth = (value?: Timestamp) => (value ? MONTH_UTC.format(utcDate(value)) : '')

const DAY_TIME = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
})

const dayTimeParts = (timestamp: Timestamp) => {
  const parts = DAY_TIME.formatToParts(timestampDate(timestamp))
  const part = (type: Intl.DateTimeFormatPartTypes) =>
    parts.find(p => p.type === type)?.value ?? ''
  return {
    day: `${part('month')} ${part('day')}`,
    time: `${part('hour')}:${part('minute')}`,
  }
}

/** An instant in the viewer's time: "Oct 8, 19:13". */
export const dayTime = (timestamp: Timestamp) => {
  const parts = dayTimeParts(timestamp)
  return `${parts.day}, ${parts.time}`
}

/** An instant in the viewer's time: "Oct 8 at 14:02". */
export const dayAtTime = (timestamp: Timestamp) => {
  const parts = dayTimeParts(timestamp)
  return `${parts.day} at ${parts.time}`
}

export const CREDIT_SOURCE_LABEL: Record<CreditSource, string> = {
  [CreditSource.UNSPECIFIED]: 'Credit',
  [CreditSource.PURCHASED]: 'Purchased credit',
  [CreditSource.INCLUDED]: 'Included with plan',
  [CreditSource.ADDED]: 'Credit added',
  [CreditSource.ROLLED_OVER]: 'Rolled over from last period',
  [CreditSource.AUTOMATIC_TOP_UP]: 'Automatic top-up',
  [CreditSource.CREDIT_NOTE]: 'Credit note',
  [CreditSource.RETURNED]: 'Credit returned',
  [CreditSource.PREVIOUS_BALANCE]: 'Previous balance',
  [CreditSource.BONUS]: 'Free credit',
  [CreditSource.OVERPAYMENT]: 'Overpayment',
}

/** What a grant is ("Included with Studio"), marked when it rolled over from a past period. */
export const grantKindLabel = (grant: Pick<CreditGrantView, 'scope' | 'source'>) => {
  const kind = grant.scope?.kindLabel || CREDIT_SOURCE_LABEL[grant.source]
  return grant.source === CreditSource.ROLLED_OVER && grant.scope?.kindLabel
    ? `${kind} · ${CREDIT_SOURCE_LABEL[CreditSource.ROLLED_OVER]}`
    : kind
}

/** The grants with credit left to spend once usage so far is paid: none scheduled or spent. */
export const liveGrants = <
  G extends Pick<CreditGrantView, 'scheduled' | 'remaining' | 'remainingAfterUsage'>,
>(
  grants: G[]
) => grants.filter(grant => !grant.scheduled && (grant.remainingAfterUsage ?? grant.remaining) > 0n)

export const fieldStyle: CSSProperties = {
  width: '100%',
  padding: '9px 12px',
  fontSize: 13.5,
  fontFamily: 'inherit',
  color: 'var(--mtp-text)',
  background: 'var(--mtp-surface)',
  border: '1px solid var(--mtp-border-2)',
  borderRadius: 'var(--mtp-r-ctrl)',
}

export const labelStyle: CSSProperties = {
  fontSize: 12,
  fontWeight: 600,
  color: 'var(--mtp-text-2)',
}

export const errorStyle: CSSProperties = { fontSize: 11.5, color: 'var(--mtp-danger)' }

/** "USD credit", or the credit unit's plural name. */
export const walletTitle = (wallet: Pick<Wallet, 'currency' | 'creditUnit'>) =>
  wallet.creditUnit ? wallet.creditUnit.pluralName : `${wallet.currency} credit`

/** One group of a wallet's credit, worded for the customer: its amount, expiry and rollover. */
export const creditGroupDetail = (
  group: CreditGroup,
  wallet: Pick<Wallet, 'currency' | 'creditUnit'>
) => {
  const fmt = (minor: bigint) => walletAmount(minor, wallet.currency, wallet.creditUnit)
  const lapse = group.nextLapseAt
    ? `${group.kind === CreditGrantKind.INCLUDED ? 'Resets' : 'Expires'} ${boundaryDay(group.nextLapseAt)}`
    : undefined
  const rollover =
    group.rollover && group.rolloverTerm
      ? group.rollover.maxAmount !== undefined
        ? `Up to ${fmt(group.rollover.maxAmount)} ${rollsOver(group.rollover.maxAmount, wallet.creditUnit)} for ${group.rolloverTerm}`
        : `All of it rolls over for ${group.rolloverTerm}`
      : undefined
  return {
    amount: fmt(group.available),
    scheduled: group.upcoming > 0n ? `+ ${fmt(group.upcoming)} scheduled` : undefined,
    notes: [group.paysFor, lapse, rollover].filter((note): note is string => Boolean(note)),
  }
}
