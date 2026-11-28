/**
 * The API key form's rules. A publishable key may name the browser origins it is accepted from:
 * typed one per line (or separated by commas), each `scheme://host[:port]` as the browser reports
 * a page's origin. The server normalizes them the same way and refuses the others.
 */

export type ApiKeyKind = 'secret' | 'publishable'

/** The origins typed, trimmed, without a trailing slash, lower-cased, each once. */
export const parseOrigins = (text: string): string[] => {
  const seen = new Set<string>()
  return text
    .split(/[\n,]/)
    .map(entry => entry.trim().replace(/\/+$/, '').toLowerCase())
    .filter(entry => {
      if (!entry || seen.has(entry)) return false
      seen.add(entry)
      return true
    })
}

/** An origin a browser can report: http(s), a host, an optional port, nothing after. */
export const isOrigin = (origin: string): boolean => {
  let url: URL
  try {
    url = new URL(origin)
  } catch {
    return false
  }
  return (
    (url.protocol === 'http:' || url.protocol === 'https:') &&
    url.hostname !== '' &&
    url.username === '' &&
    url.password === '' &&
    url.pathname === '/' &&
    url.search === '' &&
    url.hash === '' &&
    !/[/?#]/.test(origin.replace(/^https?:\/\//, ''))
  )
}

/** The typed origins that are not one; empty when the field can be sent. */
export const invalidOrigins = (text: string): string[] =>
  parseOrigins(text).filter(origin => !isOrigin(origin))

export type AccessLevel = 'none' | 'read' | 'write'

export type ApiResourceKey =
  | 'customers'
  | 'subscriptions'
  | 'invoices'
  | 'payments'
  | 'catalog'
  | 'usage'
  | 'entitlements'
  | 'webhooks'
  | 'batchJobs'

export const RESOURCES: { key: ApiResourceKey; label: string; description: string }[] = [
  {
    key: 'customers',
    label: 'Customers',
    description: 'Customers, their resources and portal links',
  },
  {
    key: 'subscriptions',
    label: 'Subscriptions',
    description: 'Subscriptions and checkout sessions',
  },
  { key: 'invoices', label: 'Invoices', description: 'Invoices and credit notes' },
  {
    key: 'payments',
    label: 'Payments & credits',
    description: 'Payments, top-ups, credit grants, wallets, budgets, issuing credit notes',
  },
  {
    key: 'catalog',
    label: 'Catalog',
    description: 'Plans, products, add-ons, coupons, features, metrics, rate cards',
  },
  { key: 'usage', label: 'Usage & events', description: 'Ingest events, read usage' },
  { key: 'entitlements', label: 'Entitlements', description: 'Entitlement checks and overrides' },
  { key: 'webhooks', label: 'Webhooks', description: 'Webhook endpoints and deliveries' },
  { key: 'batchJobs', label: 'Batch jobs', description: 'Imports and their failures' },
]

export type ResourceLevels = Record<ApiResourceKey, AccessLevel>

export const noAccess = (): ResourceLevels =>
  Object.fromEntries(RESOURCES.map(r => [r.key, 'none'])) as ResourceLevels

/** Whether a restricted key grants anything: one granting nothing is refused. */
export const grantsAnything = (levels: ResourceLevels): boolean =>
  Object.values(levels).some(level => level !== 'none')

export const LIFETIMES: { value: string; label: string; days?: number }[] = [
  { value: 'never', label: 'Never' },
  { value: '30', label: '30 days', days: 30 },
  { value: '90', label: '90 days', days: 90 },
  { value: '180', label: '180 days', days: 180 },
  { value: '365', label: '1 year', days: 365 },
]

export const ROLL_GRACE: { value: string; label: string; hours: number }[] = [
  { value: '0', label: 'Immediately', hours: 0 },
  { value: '1', label: 'In 1 hour', hours: 1 },
  { value: '24', label: 'In 24 hours', hours: 24 },
  { value: '168', label: 'In 7 days', hours: 168 },
]

const DAY_MS = 24 * 60 * 60 * 1000

export interface Expiry {
  label: string
  tone: 'muted' | 'warning' | 'expired'
}

/** How an expiry reads in the list: a warning within a week, none for a key that never expires. */
export const describeExpiry = (expiresAt: string | undefined, now: Date): Expiry | undefined => {
  if (!expiresAt) return undefined
  const left = new Date(expiresAt).getTime() - now.getTime()
  if (left <= 0) return { label: 'Expired', tone: 'expired' }
  const hours = Math.ceil(left / (60 * 60 * 1000))
  if (hours < 48) return { label: `Expires in ${hours}h`, tone: 'warning' }
  const days = Math.ceil(left / DAY_MS)
  return { label: `Expires in ${days} days`, tone: days <= 7 ? 'warning' : 'muted' }
}
