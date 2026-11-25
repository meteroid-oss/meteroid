/** One condition the customer must meet: its billing country, or one of its custom properties. */
export type Condition =
  | { kind: 'country'; countries: string[] }
  | { kind: 'property'; key: string; values: string[] }

/** The editor's state: everyone, or only customers meeting every condition listed. */
export interface EligibilityDraft {
  restricted: boolean
  conditions: Condition[]
}

/** A rule as the API carries it; an empty one lets everyone in. */
export interface Rule {
  countries: string[]
  properties: { key: string; values: string[] }[]
}

export const emptyEligibility = (): EligibilityDraft => ({ restricted: false, conditions: [] })

/** The first row offered: the country, unless a row already asks it. */
export const newCondition = (conditions: Condition[]): Condition =>
  conditions.some(c => c.kind === 'country')
    ? { kind: 'property', key: '', values: [] }
    : { kind: 'country', countries: [] }

export const toDraft = (rule: Rule | undefined): EligibilityDraft => {
  if (!rule || isOpen(rule)) return emptyEligibility()
  const conditions: Condition[] = []
  if (rule.countries.length) conditions.push({ kind: 'country', countries: [...rule.countries] })
  for (const p of rule.properties) {
    conditions.push({ kind: 'property', key: p.key, values: [...p.values] })
  }
  return { restricted: true, conditions }
}

/** The rule to save: trimmed, a row without a country, key or values dropped. */
export const fromDraft = (draft: EligibilityDraft): Rule => {
  if (!draft.restricted) return { countries: [], properties: [] }
  const countries = draft.conditions.flatMap(c => (c.kind === 'country' ? c.countries : []))
  const properties = draft.conditions
    .flatMap(c => (c.kind === 'property' ? [c] : []))
    .map(p => ({ key: p.key.trim(), values: p.values.map(v => v.trim()).filter(Boolean) }))
    .filter(p => p.key && p.values.length > 0)
  return { countries: [...new Set(countries)], properties }
}

/** A rule that lets everyone in: the same as none. */
export const isOpen = (rule: Rule | undefined): boolean =>
  !rule ||
  (rule.countries.length === 0 && !rule.properties.some(p => p.key && p.values.length > 0))

/** "a, b or c". */
const either = (items: string[], word: 'or' | 'and') =>
  items.length <= 1
    ? (items[0] ?? '')
    : `${items.slice(0, -1).join(', ')} ${word} ${items[items.length - 1]}`

const MAX_COUNTRIES = 3

const countriesPhrase = (codes: string[], name: (code: string) => string) =>
  codes.length > MAX_COUNTRIES ? `${codes.length} countries` : either(codes.map(name), 'or')

/**
 * The rule in one line: "Only customers in Brazil whose student is verified". `country` names
 * an ISO code; the code itself when unset.
 */
export function eligibilitySummary(
  rule: Rule | undefined,
  country: (code: string) => string = code => code
): string {
  if (!rule || isOpen(rule)) return 'Everyone'
  const properties = rule.properties.filter(p => p.key && p.values.length > 0)
  const phrases: string[] = []
  if (rule.countries.length) phrases.push(`in ${countriesPhrase(rule.countries, country)}`)
  if (properties.length) {
    phrases.push(
      `whose ${properties.map(p => `${p.key} is ${either(p.values, 'or')}`).join(' and ')}`
    )
  }
  return `Only customers ${phrases.join(' ')}`
}
