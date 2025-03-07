import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
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
    schema: schemas.me.emailSchema,
    defaultValues: {
      email: '',
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

  const onSubmit = async (data: z.infer<typeof schemas.me.emailSchema>) => {
    const res = await registerMut.mutateAsync({
      email: data.email,
      inviteKey: invite,
    })
    console.log(res)
    // navigate('/login')
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-6">
          <InputFormField
            name="email"
            label="Work email"
            control={methods.control}
            placeholder="you@company.com"
            id="signup-email"
          />
          <Button variant="primary" type="submit" disabled={!methods.formState.isValid}>
            Continue
          </Button>
        </div>
      </form>
    </Form>
  )
}
