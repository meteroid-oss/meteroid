import { Code, ConnectError } from '@connectrpc/connect'
import { matchRoutes } from 'react-router-dom'
import { toast } from 'sonner'

import { clearPortalToken, portalTokenScope, scopePortalToken } from '@/features/auth/portalToken'
import { getSessionToken } from '@/features/auth/session'
import router from 'router/router'

import type { Interceptor } from '@connectrpc/connect'

const loggingInterceptorSkipError = ['AbortError:', 'DOMException:']

// Mask credential-bearing fields (by name, at any depth) before they reach the
// console and session replay: connector setup carries GoCardless tokens /
// webhook secrets, Stripe `sk_` keys and Stancer `stest_`/`sprod_` keys.
const SECRET_FIELD_SUBSTRINGS = [
  'accesstoken',
  'webhooksecret',
  'apisecretkey',
  'apikey',
  'clientsecret',
]
const isSecretField = (key: string): boolean => {
  const lower = key.toLowerCase()
  return SECRET_FIELD_SUBSTRINGS.some(s => lower.includes(s))
}

// Returns the input unchanged when nothing matches; clones only along redacted
// paths. `seen` holds the current recursion path to break reference cycles.
const redactValue = (value: unknown, seen: Set<object>): unknown => {
  if (!value || typeof value !== 'object' || seen.has(value)) return value
  seen.add(value)
  try {
    if (Array.isArray(value)) {
      let out: unknown[] | null = null
      for (let i = 0; i < value.length; i++) {
        const next = redactValue(value[i], seen)
        if (next !== value[i]) {
          out = out ?? value.slice()
          out[i] = next
        }
      }
      return out ?? value
    }
    // Non-plain objects (Date, Map, protobuf messages, …) are safe here:
    // Object.keys only yields own enumerable props, and no match ⇒ no clone.
    const source = value as Record<string, unknown>
    let out: Record<string, unknown> | null = null
    for (const key of Object.keys(source)) {
      const next = isSecretField(key) ? '[redacted]' : redactValue(source[key], seen)
      if (next !== source[key]) {
        out = out ?? { ...source }
        out[key] = next
      }
    }
    return out ?? value
  } finally {
    seen.delete(value)
  }
}

const redactSecrets = (message: unknown): unknown => redactValue(message, new Set())

export const loggingInterceptor: Interceptor = next => async req => {
  try {
    const result = await next(req)
    if (import.meta.env.DEV) {
      console.log(
        `🔃 to ${req.method.name} `,
        redactSecrets(req.message),
        redactSecrets(result?.message)
      )
    }
    return result
  } catch (e) {
    const error = e
    const errorStr = String(e)

    // only error if it doesn't start with the strings in the array
    if (import.meta.env.DEV && !loggingInterceptorSkipError.some(s => errorStr.startsWith(s))) {
      console.error(`🚨 to ${req.method.name} `, redactSecrets(req.message), error)
    }

    throw error
  }
}

const errorInterceptorSkipError = [
  'TypeError:',
  'AbortError:',
  'DOMException:',
  //extra for local without metering started, TODO consider an alternative rendering of connection errors
  'ConnectError:',
]


let isRedirecting = false;

export const errorInterceptor: Interceptor = next => async req => {
  try {
    return await next(req)
  } catch (e) {
    const errorStr = String(e)

    // logout in case of authentication error (wrong url, wrong token, etc)
    if (e instanceof ConnectError) {
      // A rejected portal token (dropped by authInterceptor) says nothing about the
      // dashboard session, so no logout.
      if (e.code === 16 && req.service.typeName.startsWith('meteroid.portal')) {
        throw e
      }
      if (e.code === 16 && !isRedirecting) {
        const sessionToken = getSessionToken()
        if (sessionToken) {
          toast.error("Authentication failed. Redirecting you to login page.")
          isRedirecting = true;
          setTimeout(() => {
            setTimeout(() => {
              isRedirecting = false;
            }, 1000);
            window.location.href = "/logout"
          }, 2000)
        }
        throw e
      }

    }

    if (!errorInterceptorSkipError.some(s => errorStr.startsWith(s))) {
      toast.error(errorStr)
    }
    throw e
  }
}

export const authInterceptor: Interceptor = next => async req => {
  if (req.service.typeName.startsWith('meteroid.api') || req.service.typeName.startsWith('meteroid.admin')) {
    const matchingRoutes = matchRoutes(router.routes, window.location)
    const params = matchingRoutes?.[0]?.params
    const organizationSlug = params?.organizationSlug
    const tenantSlug = params?.tenantSlug
    const sessionToken = getSessionToken()
    organizationSlug && req.header.append('x-md-context', `${organizationSlug}/${tenantSlug || ''}`)
    sessionToken && req.header.append('Authorization', `Bearer ${sessionToken}`)
  } else if (
    req.service.typeName.startsWith('meteroid.portal') &&
    // Anonymous: a token left over from another portal page must not ride along.
    !req.service.typeName.startsWith('meteroid.portal.purchase')
  ) {
    // One scope per request: the token sent is the one dropped if the server rejects it.
    const scope = portalTokenScope(window.location.pathname)
    const token = scopePortalToken(scope, window.location.search)
    token && req.header.append('x-portal-token', token)
    try {
      return await next(req)
    } catch (e) {
      if (e instanceof ConnectError && e.code === Code.Unauthenticated) clearPortalToken(scope)
      throw e
    }
  }

  const result = await next(req)
  return result
}
