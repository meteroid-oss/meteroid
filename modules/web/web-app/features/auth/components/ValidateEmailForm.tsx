import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import { z } from 'zod'

import { useSession } from '@/features/auth/session'
import { useZodForm } from '@/hooks/useZodForm'
import { queryClient } from '@/lib/react-query'
import { schemas } from '@/lib/schemas'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { completeRegistration } from '@/rpc/api/users/v1/users-UsersService_connectquery'

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
  })

  const registerMut = useMutation(completeRegistration, {
    onSuccess: async res => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(getInstance) })
      setSession(res)
    },
  })

  const onSubmit = async (data: z.infer<typeof schemas.me.validateEmailSchema>) => {
    await registerMut.mutateAsync({
      //If validation is skipped, we get back the email from the state
      email: state,
      password: data.password,
      validationToken: token ?? '',
    })
    navigate('/login', {
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
