import { timestampDate } from '@bufbuild/protobuf/wkt'

import { LimitOrigin } from '@/rpc/portal/customer/v1/models_pb'

import { clampPct, decimalStr } from './format'
import { boundaryDay, boundaryMonth } from './walletFormat'

import type { LimitGroupUsage, UsageLimitStatus } from '@/rpc/portal/customer/v1/models_pb'

type Window = 'Daily' | 'Weekly' | 'Monthly' | 'Yearly'

const DAY_MS = 86_400_000

export const limitWindow = (limit: UsageLimitStatus): Window | undefined => {
  if (!limit.windowStart || !limit.resetsAt) return undefined
  const days = Math.round(
    (timestampDate(limit.resetsAt).getTime() - timestampDate(limit.windowStart).getTime()) / DAY_MS
  )
  if (days === 1) return 'Daily'
  if (days === 7) return 'Weekly'
  if (days >= 28 && days <= 31) return 'Monthly'
  if (days === 365 || days === 366) return 'Yearly'
  return undefined
}

/** The limit counted by this plan, e.g. the Free plan's token cap. */
export const planLimitOf = (limits: UsageLimitStatus[]) =>
  limits.find(limit => limit.origin === LimitOrigin.PLAN && limit.hard)

const ACRONYMS = new Set(['api', 'id', 'ip', 'url', 'sdk'])

/** "api_key" -> "API key". */
export const groupByLabel = (groupBy: string) =>
  groupBy
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map(word => (ACRONYMS.has(word.toLowerCase()) ? word.toUpperCase() : word.toLowerCase()))
    .join(' ')

/** What the limit counts: its metric's unit ("tokens"), else its name. */
export const limitNoun = (limit: UsageLimitStatus) =>
  limit.unitLabel?.plural || limit.name.toLowerCase()

/** "tokens per API key", or the limit's name when it is not split by group. */
export const limitSubject = (limit: UsageLimitStatus) =>
  limit.groupBy
    ? `${limitNoun(limit)} per ${groupByLabel(limit.groupBy)}`
    : limit.name.toLowerCase()

export const groupLabel = (usage: LimitGroupUsage, fallback: string) => {
  const values = Object.values(usage.group).filter(Boolean)
  return values.length > 0 ? values.join(' · ') : fallback
}

/** The groups at or over the limit, only once the server says the limit is reached (grace included). */
export const limitOverGroups = (limit: UsageLimitStatus) =>
  limit.reached ? limit.groups.filter(g => Number(g.used) >= Number(limit.limit)) : []

export const limitPct = (used: string, limit: string) => clampPct(Number(used), Number(limit))

const capitalize = (text: string) => (text ? text[0].toUpperCase() + text.slice(1) : text)

/** The banner copy for a hard limit a group has used up. */
export const limitReachedCopy = (limit: UsageLimitStatus, freePlan: boolean) => {
  const window = limitWindow(limit)
  const name = limit.name.toLowerCase()
  const title = window ? `${window} ${name} limit reached` : `${capitalize(name)} limit reached`
  const overGroups = limitOverGroups(limit)
  const subject = limit.groupBy && overGroups.length === 1 ? groupLabel(overGroups[0], '') : ''
  const amount = `${decimalStr(limit.limit)}${freePlan ? ' free' : ''} ${limitNoun(limit)}`
  const span =
    window === 'Monthly'
      ? ` for ${boundaryMonth(limit.windowStart)}`
      : window === 'Daily'
        ? ' today'
        : ''
  const resumes = limit.resetsAt ? boundaryDay(limit.resetsAt) : ''
  const text = subject
    ? `${subject} has used all ${amount}${span}.${resumes ? ` Full access for it returns on ${resumes}.` : ''}`
    : `You've used all ${amount}${span}.${resumes ? ` Full access returns on ${resumes}.` : ''}`
  return { title, text }
}
