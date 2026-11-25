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
