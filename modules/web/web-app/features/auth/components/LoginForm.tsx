import { useMutation } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import { Button, Flex, FormItem, Input } from '@ui/components'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useSession } from '@/features/auth'
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
    <form onSubmit={methods.handleSubmit(onSubmit)}>
      <Flex direction="column" gap={spaces.space7}>
        <FormItem name="email" label="Email" error={methods.formState.errors.email?.message}>
          <Input
            type="text"
            placeholder="john@acme.com"
            {...methods.register('email')}
            id="login-email"
          />
        </FormItem>
        <FormItem
          name="password"
          label="Password"
          error={methods.formState.errors.password?.message}
        >
          <Input type="password" {...methods.register('password')} id="login-password" />
        </FormItem>
        <Button variant="primary" type="submit" disabled={!methods.formState.isValid}>
          Login
        </Button>
        {error && <div className="text-base">{error}</div>}
      </Flex>
    </form>
  )
}
