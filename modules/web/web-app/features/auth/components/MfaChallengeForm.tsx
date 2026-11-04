import { create } from '@bufbuild/protobuf'
import { useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { z } from 'zod'

import { useSession } from '@/features/auth/session'
import { useAutoSubmitOtp } from '@/hooks/useAutoSubmitOtp'
import { useZodForm } from '@/hooks/useZodForm'
import { verifyMfa } from '@/rpc/api/users/v1/users-UsersService_connectquery'
import { LoginResponseSchema } from '@/rpc/api/users/v1/users_pb'

const mfaSchema = z.object({
  code: z
    .string()
    .min(6, 'Enter the 6-digit code from your authenticator (or a recovery code).')
    .max(20),
})

export const MfaChallengeForm = () => {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const [, setSession] = useSession()
  const [error, setError] = useState<string>()
  const challengeToken = searchParams.get('token') ?? ''
  const returnUrl = searchParams.get('returnUrl')

  const methods = useZodForm({
    schema: mfaSchema,
    defaultValues: { code: '' },
    mode: 'onSubmit',
  })

  const verifyMut = useMutation(verifyMfa, {
    onSuccess: data => {
      setSession(create(LoginResponseSchema, { token: data.token, user: data.user }))
      setTimeout(() => {
        if (returnUrl && returnUrl.startsWith('/')) {
          navigate(returnUrl)
        } else {
          navigate('/')
        }
      }, 50)
    },
  })

  const onSubmit = (data: z.infer<typeof mfaSchema>) => {
    setError(undefined)
    return verifyMut
      .mutateAsync({
        mfaChallengeToken: challengeToken,
        code: data.code.trim(),
      })
      .catch(err => {
        const raw = err?.rawMessage
        setError(raw ?? 'Invalid code. Try again or use a recovery code.')
      })
  }

  useAutoSubmitOtp(methods.watch('code'), verifyMut.isPending, () =>
    void methods.handleSubmit(onSubmit)(),
  )

  if (!challengeToken) {
    return (
      <div className="text-base text-destructive">
        Missing challenge token. Please <a className="underline" href="/login">log in again</a>.
      </div>
    )
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-2">
          <div className="text-muted-foreground text-sm">
            Enter the 6-digit code from your authenticator app, or one of your one-time recovery
            codes.
          </div>
          <InputFormField
            name="code"
            label="Authenticator code"
            control={methods.control}
            placeholder="123456"
            autoComplete="one-time-code"
            autoFocus
          />
          <Button
            variant="primary"
            type="submit"
            className="mt-3"
            disabled={!methods.formState.isValid || verifyMut.isPending}
          >
            Verify
          </Button>
          {error && <div className="text-sm text-destructive">{error}</div>}
        </div>
      </form>
    </Form>
  )
}
