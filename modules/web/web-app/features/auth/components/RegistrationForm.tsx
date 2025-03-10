import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Form, Input } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { FormItem } from '@ui/components/legacy'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useSession } from '@/features/auth/session'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { completeRegistration } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const RegistrationForm = ({ invite }: { invite?: string }) => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [, setSession] = useSession()

  const methods = useZodForm({
    schema: schemas.me.emailPasswordSchema,
    defaultValues: {
      email: '',
      password: '',
    },
  })

  const registerMut = useMutation(completeRegistration, {
    onSuccess: async res => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(getInstance) })
      setSession(res)
    },
    onError: err => {
      methods.setError('email', {
        message: err.rawMessage ?? 'An error occurred, please try again later.',
      })
    },
  })

  const onSubmit = async (data: z.infer<typeof schemas.me.emailPasswordSchema>) => {
    await registerMut.mutateAsync({
      email: data.email,
      password: data.password,
      inviteKey: invite,
    })
    navigate('/login', {
      state: 'accountCreated',
    })
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-7">
          <FormItem name="email" label="Email" error={methods.formState.errors.email?.message}>
            <Input
              id="register-email"
              type="text"
              placeholder="john@acme.com"
              {...methods.register('email')}
            />
          </FormItem>
          <FormItem
            name="password"
            label="Password"
            error={methods.formState.errors.password?.message}
          >
            <Input id="register-pwd" type="password" {...methods.register('password')} />
          </FormItem>
          <Button variant="primary" type="submit" disabled={!methods.formState.isValid}>
            Create my account
          </Button>
        </div>
      </form>
    </Form>
  )
}
