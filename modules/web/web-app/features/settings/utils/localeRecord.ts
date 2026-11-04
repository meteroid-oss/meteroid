// Helpers for a per-locale record edited through a pill strip: which locales get a
// pill, which can still be added, and how the record is seeded and pruned. Generic
// over the entry shape so invoice footers and email copy share one behaviour.

import { useEffect, useState } from 'react'

import { INVOICE_LANGUAGE_DEFAULT, INVOICE_LANGUAGE_OPTIONS } from '@/components/InvoiceLanguageSelect'

/** Real locales only — the abstract "default" sentinel never gets a pill. */
const LOCALES: string[] = INVOICE_LANGUAGE_OPTIONS.map(o => o.value).filter(
  v => v !== INVOICE_LANGUAGE_DEFAULT
)

export const localeLabel = (value: string): string =>
  INVOICE_LANGUAGE_OPTIONS.find(o => o.value === value)?.label ?? value

export const isBlank = (s?: string): boolean => !s || s.trim().length === 0

// Pills in display order: home, the effective locale when it differs, then every locale
// holding text of its own or added and not yet filled in.
const visibleLocales = <T>(
  homeLocale: string,
  effectiveLocale: string,
  record: Record<string, T | undefined> | undefined,
  added: string[],
  hasText: (entry?: T) => boolean
): string[] => {
  const pinned = effectiveLocale === homeLocale ? [homeLocale] : [homeLocale, effectiveLocale]
  const rest = LOCALES.filter(
    locale => !pinned.includes(locale) && (added.includes(locale) || hasText(record?.[locale]))
  )
  return [...pinned, ...rest]
}

/** Seed every supported locale with an empty entry, then overlay what is stored. */
export const seedLocaleRecord = <T>(
  empty: () => T,
  byLocale?: Record<string, T | undefined>
): Record<string, T> => {
  const record: Record<string, T> = {}
  for (const locale of LOCALES) record[locale] = empty()
  for (const [locale, entry] of Object.entries(byLocale ?? {})) {
    if (entry !== undefined) record[locale] = entry
  }
  return record
}

/** Drop every locale without text of its own, mirroring the backend's pruning. */
export const pruneLocaleRecord = <T>(
  record: Record<string, T | undefined> | undefined,
  hasText: (entry?: T) => boolean
): Record<string, T> => {
  const pruned: Record<string, T> = {}
  for (const [locale, entry] of Object.entries(record ?? {})) {
    if (entry !== undefined && hasText(entry)) pruned[locale] = entry
  }
  return pruned
}

interface LocaleTabsArgs<T> {
  homeLocale: string
  effectiveLocale?: string
  record: Record<string, T | undefined> | undefined
  hasText: (entry?: T) => boolean
}

// View-only pill state over a per-locale record: a pill that disappears (text cleared,
// locale no longer effective) hands the view back to the effective locale.
export const useLocaleTabs = <T>({ homeLocale, effectiveLocale, record, hasText }: LocaleTabsArgs<T>) => {
  const effective = effectiveLocale ?? homeLocale
  const [active, setActive] = useState<string>(effective)
  const [added, setAdded] = useState<string[]>([])

  const tabs = visibleLocales(homeLocale, effective, record, added, hasText)
  const addable = LOCALES.filter(l => !tabs.includes(l))
  const shown = tabs.includes(active) ? active : effective

  useEffect(() => {
    if (shown !== active) setActive(shown)
  }, [shown, active])

  const add = (locale: string) => {
    setAdded(langs => (langs.includes(locale) ? langs : [...langs, locale]))
    setActive(locale)
  }
  const forget = (locale: string) => setAdded(langs => langs.filter(l => l !== locale))
  const removable = (locale: string) =>
    locale !== homeLocale && locale !== effective && (added.includes(locale) || hasText(record?.[locale]))

  return { tabs, addable, shown, setActive, add, forget, removable }
}
