// Which languages an invoicing entity sends customer emails in.

import {
  INVOICE_LANGUAGE_DEFAULT,
  INVOICE_LANGUAGE_OPTIONS,
} from '@/components/InvoiceLanguageSelect'

import type { EmailSettings } from '@/rpc/api/customeremails/v1/models_pb'

/** Template locales the email renderer supports, in display order. */
export const TEMPLATE_LOCALES: string[] = INVOICE_LANGUAGE_OPTIONS.map(o => o.value).filter(
  v => v !== INVOICE_LANGUAGE_DEFAULT
)

/**
 * The entity's default locale first, then the operator's extras, deduped. `fallback`
 * stands in for the default while the settings load (or when the server omits it).
 */
export const enabledLocales = (
  settings?: Pick<EmailSettings, 'effectiveLocale' | 'extraLocales'>,
  fallback?: string
): string[] => {
  const all: (string | undefined)[] = settings
    ? [settings.effectiveLocale || fallback, ...(settings.extraLocales ?? [])]
    : [fallback]
  return all.filter((l, i): l is string => !!l && all.indexOf(l) === i)
}
