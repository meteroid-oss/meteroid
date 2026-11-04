// Helpers bridging the per-locale invoice footer between the generated proto
// (`FooterTranslations.byLocale`) and the flat record the form edits.

export interface FooterTranslationForm {
  info?: string
  legal?: string
}

/** Per-locale footer overrides as held in the form: locale -> { info, legal }. */
export type FooterTranslationsForm = Record<string, FooterTranslationForm>

const isBlank = (s?: string): boolean => !s || s.trim().length === 0

/** A locale carries a real override once either field has non-blank text. */
export const hasFooterOverride = (t?: FooterTranslationForm): boolean =>
  !isBlank(t?.info) || !isBlank(t?.legal)

/**
 * The footer languages that get a tab, in display order.
 *
 * - `homeLocale`, always and first: it writes the untagged base footer, which any
 *   other language prints wherever its own field is empty.
 * - `invoiceLocale`, always: the language invoices are issued in. Left empty while
 *   it differs from home, every invoice prints the fallback — the one empty
 *   language worth keeping in view.
 * - every other language with text of its own, or added and not yet filled in.
 *
 * Derived on every render rather than stored, so changing the invoice language
 * re-derives it: the old language keeps its tab only if it holds text.
 */
export const visibleFooterLocales = (
  locales: string[],
  homeLocale: string,
  invoiceLocale: string,
  translations: FooterTranslationsForm | undefined,
  added: string[]
): string[] => {
  const pinned = invoiceLocale === homeLocale ? [homeLocale] : [homeLocale, invoiceLocale]
  const rest = locales.filter(
    locale =>
      !pinned.includes(locale) &&
      (added.includes(locale) || hasFooterOverride(translations?.[locale]))
  )
  return [...pinned, ...rest]
}

/**
 * Generated `FooterTranslations.byLocale` -> form record. Strips the protobuf
 * message down to `{ info, legal }` (dropping `$typeName` and friends) and
 * defaults missing fields to '' so the textareas stay controlled.
 */
export const fromByLocale = (
  byLocale?: Record<string, FooterTranslationForm | undefined>
): FooterTranslationsForm => {
  const record: FooterTranslationsForm = {}
  for (const [locale, t] of Object.entries(byLocale ?? {})) {
    record[locale] = { info: t?.info ?? '', legal: t?.legal ?? '' }
  }
  return record
}

/**
 * Seed every supported locale with empty text, then overlay whatever the entity
 * stored, so an empty language still binds to a defined value.
 *
 * `homeLocale` — the entity's country language, not the one it issues documents
 * in — adopts the untagged base footer, that being the language it is written in.
 * An override already on that locale wins, and the base text it shadowed is lost.
 */
export const seedFooterForm = (
  locales: string[],
  byLocale?: Record<string, FooterTranslationForm | undefined>,
  homeLocale?: string,
  baseInfo?: string,
  baseLegal?: string
): FooterTranslationsForm => {
  const seeded: FooterTranslationsForm = {}
  for (const locale of locales) seeded[locale] = { info: '', legal: '' }
  const form = { ...seeded, ...fromByLocale(byLocale) }
  if (homeLocale) {
    const home = form[homeLocale] ?? { info: '', legal: '' }
    form[homeLocale] = {
      info: isBlank(home.info) ? (baseInfo ?? '') : home.info,
      legal: isBlank(home.legal) ? (baseLegal ?? '') : home.legal,
    }
  }
  return form
}

/**
 * Form record -> `byLocale` init shape for the update payload. Drops blank
 * fields and any locale left fully empty, mirroring the backend's own pruning.
 * Every locale is written explicitly, the home one included, so the payload never
 * depends on which language leads.
 */
export const toByLocale = (
  record?: Record<string, FooterTranslationForm | undefined>
): Record<string, FooterTranslationForm> => {
  const byLocale: Record<string, FooterTranslationForm> = {}
  for (const [locale, t] of Object.entries(record ?? {})) {
    const entry: FooterTranslationForm = {}
    if (!isBlank(t?.info)) entry.info = t?.info
    if (!isBlank(t?.legal)) entry.legal = t?.legal
    if (entry.info !== undefined || entry.legal !== undefined) {
      byLocale[locale] = entry
    }
  }
  return byLocale
}
