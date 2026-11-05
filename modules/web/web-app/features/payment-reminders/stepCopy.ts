// Which enabled locales a reminder step's custom copy still lacks. Mirrors the server rule:
// a step with custom copy needs it in every enabled locale; one on the built-in copy, none.

import { useEffect, useState } from 'react'

import { TEMPLATE_LOCALES } from '@/features/customer-emails/locales'
import { isBlank, localeLabel } from '@/features/settings/utils/localeRecord'

export type Copy = { subject: string; body: string }
type CopyRecord = Record<string, Partial<Copy> | undefined> | undefined

export const emptyCopy = (): Copy => ({ subject: '', body: '' })
export const hasCopy = (c?: Partial<Copy>) => !isBlank(c?.subject) || !isBlank(c?.body)
const isComplete = (c?: Partial<Copy>) => !isBlank(c?.subject) && !isBlank(c?.body)

/** True when no locale holds text: the step sends the built-in reminder copy. */
export const usesBuiltInCopy = (content: CopyRecord): boolean =>
  !Object.values(content ?? {}).some(hasCopy)

/** Enabled locales a step's custom copy lacks a subject or body in. */
export const missingCopyLocales = (content: CopyRecord, locales: string[]): string[] =>
  usesBuiltInCopy(content) ? [] : locales.filter(l => !isComplete(content?.[l]))

/** Union of the missing locales over a stored sequence's steps, in `locales` order. */
export const sequenceMissingLocales = (
  steps: { content?: { byLocale: Record<string, Partial<Copy>> } }[],
  locales: string[]
): string[] => {
  const missing = new Set(steps.flatMap(s => missingCopyLocales(s.content?.byLocale, locales)))
  return locales.filter(l => missing.has(l))
}

export const localeListLabel = (locales: string[]): string => locales.map(localeLabel).join(', ')

// Tab state for a step's copy: every enabled locale (default first), then any other
// locale still holding copy, shown as "not enabled" until the operator removes it.
export const useStepLocaleTabs = ({
  locales,
  content,
}: {
  locales: string[]
  content: CopyRecord
}) => {
  const home = locales[0]
  const [active, setActive] = useState<string>(home)

  const dormant = TEMPLATE_LOCALES.filter(l => !locales.includes(l) && hasCopy(content?.[l]))
  const tabs = [...locales, ...dormant]
  const shown = tabs.includes(active) ? active : home

  useEffect(() => {
    if (shown !== active) setActive(shown)
  }, [shown, active])

  const isEnabled = (locale: string) => locales.includes(locale)

  return { tabs, shown, setActive, isEnabled }
}

export type StepLocaleTabs = ReturnType<typeof useStepLocaleTabs>
