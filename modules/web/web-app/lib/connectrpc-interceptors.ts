import { ConnectError } from '@connectrpc/connect'
import { matchRoutes } from 'react-router-dom'
import { toast } from 'sonner'

import { getSessionToken } from '@/features/auth/session'
import router from 'router/router'

import type { Interceptor } from '@connectrpc/connect'

const loggingInterceptorSkipError = ['AbortError:', 'DOMException:']

// Shallow-scan message props by name and mask credential-bearing fields before
// they reach the console (and PostHog session replay). Payment connector setup
// carries GoCardless access tokens / webhook secrets and Stripe `sk_` keys.
const SECRET_FIELD_SUBSTRINGS = [
  'accesstoken',
  'webhooksecret',
  'apisecretkey',
  'apikey',
  'clientsecret',
]
const redactSecrets = (message: unknown): unknown => {
  if (!message || typeof message !== 'object') return message
  const source = message as Record<string, unknown>
  let redacted: Record<string, unknown> | null = null
  for (const key of Object.keys(source)) {
    const lower = key.toLowerCase()
    if (SECRET_FIELD_SUBSTRINGS.some(s => lower.includes(s))) {
      redacted = redacted ?? { ...source }
      redacted[key] = '[redacted]'
    }
  }
  return redacted ?? message
}

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

let isRedirecting = false

export const errorInterceptor: Interceptor = next => async req => {
  try {
    return await next(req)
  } catch (e) {
    const errorStr = String(e)

    // logout in case of authentication error (wrong url, wrong token, etc)
    if (e instanceof ConnectError) {
      if (e.code === 16 && !isRedirecting) {
        const sessionToken = getSessionToken()
        if (sessionToken) {
          toast.error('Authentication failed. Redirecting you to login page.')
          isRedirecting = true
          setTimeout(() => {
            setTimeout(() => {
              isRedirecting = false
            }, 1000)
            window.location.href = '/logout'
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
  if (
    req.service.typeName.startsWith('meteroid.api') ||
    req.service.typeName.startsWith('meteroid.admin')
  ) {
    const matchingRoutes = matchRoutes(router.routes, window.location)
    const params = matchingRoutes?.[0]?.params
    const organizationSlug = params?.organizationSlug
    const tenantSlug = params?.tenantSlug
    const sessionToken = getSessionToken()
    organizationSlug && req.header.append('x-md-context', `${organizationSlug}/${tenantSlug || ''}`)
    sessionToken && req.header.append('Authorization', `Bearer ${sessionToken}`)
  } else if (req.service.typeName.startsWith('meteroid.portal')) {
    // Persist the `?token=` to sessionStorage so it survives a third-party
    // round-trip that returns without the token (e.g. GoCardless mandate flow).
    const urlToken = new URLSearchParams(window.location.search).get('token')
    if (urlToken) {
      sessionStorage.setItem('portal-token', urlToken)
    }
    const token = urlToken ?? sessionStorage.getItem('portal-token')
    token && req.header.append('x-portal-token', token)
  }

  const result = await next(req)
  return result
}
