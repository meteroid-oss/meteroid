import { create } from '@bufbuild/protobuf'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import { z } from 'zod'

import { useSession } from '@/features/auth/session'
import { useZodForm } from '@/hooks/useZodForm'
import { queryClient } from '@/lib/react-query'
import { schemas } from '@/lib/schemas'
import { INVITE_TOKEN_KEY } from '@/pages/invite/acceptInvite'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { completeRegistration } from '@/rpc/api/users/v1/users-UsersService_connectquery'
import { LoginResponseSchema } from '@/rpc/api/users/v1/users_pb'

import { RETURN_URL_KEY } from './RegistrationForm'

export const ValidateEmailForm = () => {
  const navigate = useNavigate()
  const [, setSession] = useSession()

  const [searchParams] = useSearchParams()

  const { state } = useLocation()

  const token = searchParams.get('token')

  const methods = useZodForm({
    schema: schemas.me.validateEmailSchema,
    defaultValues: {
      password: '',
      confirmPassword: '',
    },
    mode: 'onTouched',
  })

  const registerMut = useMutation(completeRegistration, {
    onSuccess: async res => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: getInstance,
        cardinality: 'finite'
      }) })
      setSession(create(LoginResponseSchema, { token: res.token, user: res.user }))
    },
  })

  const onSubmit = async (data: z.infer<typeof schemas.me.validateEmailSchema>) => {
    const pendingInviteKey = sessionStorage.getItem(INVITE_TOKEN_KEY) ?? undefined

    await registerMut.mutateAsync({
      email: state,
      password: data.password,
      validationToken: token ?? '',
      inviteKey: pendingInviteKey,
    })

    sessionStorage.removeItem(INVITE_TOKEN_KEY)

    const pendingReturnUrl = sessionStorage.getItem(RETURN_URL_KEY)
    sessionStorage.removeItem(RETURN_URL_KEY)

    // Navigate to login with returnUrl if available
    const loginPath = pendingReturnUrl ? `/login?returnUrl=${encodeURIComponent(pendingReturnUrl)}` : '/login'
    navigate(loginPath, {
      state: 'accountCreated',
    })
  }
  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-3">
          <InputFormField
            name="password"
            label="Password"
            control={methods.control}
            placeholder="Create password"
            showPasswordToggle
            autoFocus
          />
          <InputFormField
            name="confirmPassword"
            label="Confirm Password"
            control={methods.control}
            placeholder="Re-enter password"
            showPasswordToggle
          />
          <Button
            variant="secondary"
            type="submit"
            disabled={!methods.formState.isValid}
            className="mt-2"
          >
            Continue
          </Button>
        </div>
      </form>
    </Form>
  )
}
