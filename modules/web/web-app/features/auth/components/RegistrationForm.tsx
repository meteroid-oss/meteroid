import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { INVITE_TOKEN_KEY } from '@/pages/invite/acceptInvite'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { initRegistration } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const RegistrationForm = ({ invite }: { invite?: string }) => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const methods = useZodForm({
    schema: schemas.me.emailSchema,
    defaultValues: {
      email: '',
    },
    mode: 'onSubmit',
  })

  const registerMut = useMutation(initRegistration, {
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey(getInstance) })
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

    // Clear invite token from sessionStorage after successful registration init
    // The invite will be handled during completeRegistration
    if (invite) {
      sessionStorage.removeItem(INVITE_TOKEN_KEY)
    }

    res.validationRequired
      ? navigate('/check-inbox', {
          state: data.email,
        })
      : navigate('/validate-email', {
          state: data.email,
        })
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-6">
          <InputFormField
            autoFocus
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
