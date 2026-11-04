import { INVITE_TOKEN_KEY } from '@/features/auth/invite'
import { env } from '@/lib/env'

export interface SsoDiscovery {
  connectionId: string
  provider: string
  enforced: boolean
}

// Resolve the org IdP that owns an email's domain. Returns null when no SSO is
// configured (HTTP 404) or on any error, so callers fall back to password.
export const discoverSso = async (email: string): Promise<SsoDiscovery | null> => {
  const trimmed = email.trim()
  if (!trimmed) return null
  try {
    const res = await fetch(
      `${env.meteroidRestApiUri}/sso/discover?email=${encodeURIComponent(trimmed)}`
    )
    if (!res.ok) return null
    const body = (await res.json()) as {
      connection_id: string
      provider: string
      enforced: boolean
    }
    return { connectionId: body.connection_id, provider: body.provider, enforced: body.enforced }
  } catch {
    return null
  }
}

// Hand off to the backend SSO handshake (which redirects to the IdP). An
// inviteKey, when present, is consumed at callback so the member joins with the
// invited role instead of a plain JIT membership.
export const startSso = (connectionId: string, inviteKey?: string | null) => {
  const params = new URLSearchParams({ connection_id: connectionId })
  if (inviteKey && inviteKey.trim()) {
    params.set('invite_key', inviteKey.trim())
    sessionStorage.removeItem(INVITE_TOKEN_KEY)
  }
  window.location.href = `${env.meteroidRestApiUri}/sso/authorize?${params.toString()}`
}
