import { useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { initResetPassword } from '@/rpc/api/users/v1/users-UsersService_connectquery'
import { useNavigate } from 'react-router-dom'

export const ForgotPasswordForm = () => {
  const navigate = useNavigate()

  const methods = useZodForm({
    schema: schemas.me.emailSchema,
    defaultValues: {
      email: '',
    },
  })

  const registerMut = useMutation(initResetPassword, {
    onError: err => {
      methods.setError('email', {
        message: err.rawMessage ?? 'An error occurred, please try again later.',
      })
    },
  })

  const onSubmit = async (data: z.infer<typeof schemas.me.emailSchema>) => {
    await registerMut.mutateAsync({
      email: data.email,
    })
    navigate('/check-inbox-password', { state: data.email })
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-2">
          <InputFormField
            name="email"
            label="Email address"
            control={methods.control}
            placeholder="acme@company.com"
            id="reset-password"
          />
          <Button
            variant="primary"
            type="submit"
            className="mt-3"
            disabled={!methods.formState.isValid}
          >
            Reset password
          </Button>
          <Button variant="ghost" onClick={() => navigate('/login')}>
            Back to login
          </Button>
        </div>
      </form>
    </Form>
  )
}
