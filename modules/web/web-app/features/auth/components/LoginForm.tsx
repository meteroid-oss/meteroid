import { useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useSession } from '@/features/auth/session'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { login } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const LoginForm = () => {
  const navigate = useNavigate()
  const [, setSession] = useSession()
  const [error, setError] = useState<string>()
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
      setSession(data)
      // Small delay to ensure session is persisted before navigation
      setTimeout(() => {
        navigate('/')
      }, 50)
    },
  })

  const onSubmit = async (data: z.infer<typeof schemas.me.emailPasswordSchema>) => {
    return loginMutation
      .mutateAsync({
        email: data.email,
        password: data.password,
      })
      .catch(error => {
        console.log('error', { error })
        const rawMessage = error?.rawMessage
        rawMessage
          ? setError(rawMessage)
          : setError('Unable to identify, please verify your credentials')
      })
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-2">
          <InputFormField
            name="email"
            label="Email"
            control={methods.control}
            placeholder="you@company.com"
            id="login-email"
            autoFocus
          />

          <InputFormField
            name="password"
            label="Password"
            control={methods.control}
            placeholder="Enter your password"
            type="password"
            id="login-password"
            rightLabel={
              <Link to="/forgot-password" className="text-muted-foreground text-xs underline">
                Forgot password?
              </Link>
            }
          />

          <Button
            variant="primary"
            type="submit"
            className="mt-3"
            disabled={!methods.formState.isValid}
          >
            Login
          </Button>
          {error && <div className="text-base">{error}</div>}
        </div>
      </form>
    </Form>
  )
}
