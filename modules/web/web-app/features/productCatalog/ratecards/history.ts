import { format, parseISO } from 'date-fns'

import type { RateCardEntry } from '@/rpc/api/ratecards/v1/models_pb'

/** "Mar 1" this year, "Mar 1, 2025" another. */
export const shortDay = (iso: string) => {
  const day = parseISO(iso)
  return format(day, day.getFullYear() === new Date().getFullYear() ? 'MMM d' : 'MMM d, yyyy')
}

/** Where an entry stands on a day: billed and over, the one in force, or still to come. */
export type Standing = 'past' | 'current' | 'scheduled'

export interface Step {
  id: string
  from: string
  /** Unset: the key is removed in the currency from that day. */
  rate?: string
  standing: Standing
}

/** A key's rates in one currency (or a credit card's unit) over time, oldest first. */
export interface KeyHistory {
  id: string
  key: string[]
  anyOther: boolean
  currency?: string
  steps: Step[]
}

const isAnyOther = (key: string[]) => key.every(pattern => pattern === '*')

/** "gpt-5 · input", "any · cached"; Anything else for the key matching anything. */
export const keyLabel = (key: string[]) =>
  isAnyOther(key) ? 'Anything else' : key.map(p => (p === '*' ? 'any' : p)).join(' · ')

/**
 * Each key and currency's entries on `today`: every rate it had, the one in force and those
 * scheduled, removals included. By key, Anything else last, then currency.
 */
export function rateHistory(entries: RateCardEntry[], today: string): KeyHistory[] {
  const byKey = new Map<string, KeyHistory>()
  for (const entry of entries) {
    const id = `${entry.key.join('\u0001')}\u0002${entry.currency ?? ''}`
    let history = byKey.get(id)
    if (!history) {
      history = {
        id,
        key: entry.key,
        anyOther: entry.anyOther || isAnyOther(entry.key),
        currency: entry.currency,
        steps: [],
      }
      byKey.set(id, history)
    }
    history.steps.push({
      id: entry.id,
      from: entry.effectiveFrom,
      rate: entry.rate,
      standing: entry.effectiveFrom > today ? 'scheduled' : 'past',
    })
  }
  for (const history of byKey.values()) {
    history.steps.sort((a, b) => a.from.localeCompare(b.from))
    const current = history.steps.filter(step => step.standing === 'past').at(-1)
    if (current) current.standing = 'current'
  }
  return [...byKey.values()].sort(
    (a, b) =>
      Number(a.anyOther) - Number(b.anyOther) ||
      a.key.join(',').localeCompare(b.key.join(',')) ||
      (a.currency ?? '').localeCompare(b.currency ?? '')
  )
}

/** The histories whose key holds `text`, any case; all of them for no text. */
export function filterHistory(histories: KeyHistory[], text: string): KeyHistory[] {
  const needle = text.trim().toLowerCase()
  if (!needle) return histories
  return histories.filter(h => keyLabel(h.key).toLowerCase().includes(needle))
}

/**
 * What a batch did to a scheduled entry it deleted: a rate (or removal) of its key, currency and
 * day took its place, or it was withdrawn.
 */
export function withdrawal(
  rates: { key: string[]; currency?: string }[],
  effectiveFrom: string | undefined,
  entry: { key: string[]; currency?: string; effectiveFrom: string }
): 'replaced' | 'withdrawn' {
  const sameKey = (key: string[]) =>
    key.length === entry.key.length && key.every((p, i) => p === entry.key[i])
  return effectiveFrom === entry.effectiveFrom &&
    rates.some(rate => sameKey(rate.key) && rate.currency === entry.currency)
    ? 'replaced'
    : 'withdrawn'
}
