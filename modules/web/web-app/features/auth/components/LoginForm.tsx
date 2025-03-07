import { useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
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
  })

  const loginMutation = useMutation(login, {
    onSuccess: data => {
      setSession(data)
      navigate('/')
    },
  })

  const onSubmit = async (data: z.infer<typeof schemas.me.emailPasswordSchema>) => {
    return loginMutation
      .mutateAsync({
        email: data.email,
        password: data.password,
      })
      .catch(e => {
        console.log('error', e)
        setError('Unable to identify, please verify your credentials')
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
          />

          <InputFormField
            name="password"
            label="Password"
            control={methods.control}
            placeholder="Enter your password"
            type="password"
            id="login-password"
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
