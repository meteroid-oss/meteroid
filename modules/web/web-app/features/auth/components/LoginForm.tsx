import { useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'
import { z } from 'zod'

import { INVITE_TOKEN_KEY } from '@/features/auth/invite'
import { useSession } from '@/features/auth/session'
import { SsoDiscovery, discoverSso, startSso } from '@/features/auth/sso'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { login } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const LoginForm = () => {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const [, setSession] = useSession()
  const [error, setError] = useState<string>()
  // Email-first: reveal the password (and any SSO option) only after the email
  // is known, so the org's IdP can claim the login before a password is typed.
  const [stage, setStage] = useState<'email' | 'password'>('email')
  const [sso, setSso] = useState<SsoDiscovery | null>(null)
  const [checking, setChecking] = useState(false)
  const returnUrl = searchParams.get('returnUrl')
  const methods = useZodForm({
    schema: schemas.me.emailPasswordSchema,
    defaultValues: {
      email: '',
      password: '',
    },
    mode: 'onSubmit',
  })

  const loginMutation = useMutation(login, {
    onSuccess: data => {
      if (data.mfaChallengeToken) {
        const params = new URLSearchParams({ token: data.mfaChallengeToken })
        if (returnUrl && returnUrl.startsWith('/')) {
          params.set('returnUrl', returnUrl)
        }
        navigate(`/mfa-challenge?${params.toString()}`)
        return
      }

      setSession(data)

      // Check if there's a pending invite token
      const pendingInvite = sessionStorage.getItem(INVITE_TOKEN_KEY)

      // Small delay to ensure session is persisted before navigation
      setTimeout(() => {
        if (pendingInvite) {
          navigate(`/invite-authenticated?token=${pendingInvite}`)
        } else if (returnUrl && returnUrl.startsWith('/')) {
          // Redirect to returnUrl if it's a valid relative path
          navigate(returnUrl)
        } else {
          navigate('/')
        }
      }, 50)
    },
  })

  const pendingInvite = () => sessionStorage.getItem(INVITE_TOKEN_KEY)

  // Resolve the org IdP from the typed email. Enforced SSO redirects straight
  // away; otherwise reveal the password (plus an optional SSO button).
  const onContinue = async () => {
    setError(undefined)
    if (!(await methods.trigger('email'))) return
    const email = methods.getValues('email').trim()
    setChecking(true)
    const found = await discoverSso(email)
    setChecking(false)
    setSso(found)
    if (found?.enforced) {
      startSso(found.connectionId, pendingInvite())
      return
    }
    setStage('password')
    setTimeout(() => methods.setFocus('password'), 0)
  }

  const onSubmit = async (data: z.infer<typeof schemas.me.emailPasswordSchema>) => {
    return loginMutation
      .mutateAsync({
        email: data.email,
        password: data.password,
      })
      .catch(async error => {
        const rawMessage = error?.rawMessage
        setError(rawMessage ?? 'Unable to identify, please verify your credentials')
        // The password was rejected because the org mandates SSO (or discovery
        // missed it earlier). Re-resolve the IdP so we can offer the SSO path
        // rather than leaving the user stuck on the error.
        const found = await discoverSso(data.email)
        if (found) setSso(found)
      })
  }

  return (
    <Form {...methods}>
      <form
        onSubmit={e => {
          e.preventDefault()
          if (stage === 'email') void onContinue()
          else void methods.handleSubmit(onSubmit)()
        }}
      >
        <div className="flex flex-col gap-2">
          <InputFormField
            name="email"
            label="Email"
            control={methods.control}
            placeholder="you@company.com"
            id="login-email"
            autoComplete="username"
            autoFocus
            // Locked once revealed: changing the domain must re-run discovery
            // (via "Use a different email"), so an SSO-enforced address can't be
            // submitted down the password path into a dead end.
            readOnly={stage === 'password'}
            rightLabel={
              stage === 'password' ? (
                <button
                  type="button"
                  className="text-muted-foreground text-xs underline"
                  onClick={() => {
                    setStage('email')
                    setSso(null)
                    setError(undefined)
                    setTimeout(() => methods.setFocus('email'), 0)
                  }}
                >
                  Use a different email
                </button>
              ) : undefined
            }
          />

          {/* Kept mounted (visually hidden until revealed) so password managers
              can still detect and fill the credential pair. */}
          <div className={stage === 'password' ? '' : 'sr-only'} aria-hidden={stage !== 'password'}>
            <InputFormField
              name="password"
              label="Password"
              control={methods.control}
              placeholder="Enter your password"
              type="password"
              id="login-password"
              autoComplete="current-password"
              tabIndex={stage === 'password' ? undefined : -1}
              rightLabel={
                <Link to="/forgot-password" className="text-muted-foreground text-xs underline">
                  Forgot password?
                </Link>
              }
            />
          </div>

          {stage === 'email' ? (
            <Button variant="primary" type="submit" className="mt-3" disabled={checking}>
              {checking ? 'Checking…' : 'Continue'}
            </Button>
          ) : (
            <>
              <Button
                variant="primary"
                type="submit"
                className="mt-3"
                disabled={loginMutation.isPending}
              >
                {loginMutation.isPending ? 'Signing in…' : 'Login'}
              </Button>
              {sso && (
                <Button
                  variant="secondary"
                  type="button"
                  className="mt-1"
                  onClick={() => startSso(sso.connectionId, pendingInvite())}
                >
                  Continue with SSO
                </Button>
              )}
            </>
          )}
          {error && <div className="text-base">{error}</div>}
        </div>
      </form>
    </Form>
  )
}
