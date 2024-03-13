import { useMutation } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import { Button, Form, InputFormField } from '@md/ui'
import { Flex } from '@ui/components/legacy'
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
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <Flex direction="column" gap={spaces.space7}>
          <InputFormField
            name="email"
            label="Email"
            control={methods.control}
            placeholder="john@acme.com"
            id="login-email"
          />

          <InputFormField
            name="password"
            label="Password"
            control={methods.control}
            placeholder="Password"
            type="password"
            id="login-password"
          />

          <Button variant="primary" type="submit" disabled={!methods.formState.isValid}>
            Login
          </Button>
          {error && <div className="text-base">{error}</div>}
        </Flex>
      </form>
    </Form>
  )
}
