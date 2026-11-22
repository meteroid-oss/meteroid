// Sign-in redirects carry the token in the fragment, which never reaches a server log or Referer.
export const readRedirectToken = (location: Pick<Location, 'hash' | 'search'>): string | null =>
  new URLSearchParams(location.hash.replace(/^#/, '')).get('token') ??
  new URLSearchParams(location.search).get('token')
