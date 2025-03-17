import { useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { resetPassword } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const ResetPasswordForm = () => {
  const navigate = useNavigate()

  const [searchParams] = useSearchParams()

  const token = searchParams.get('token')

  const methods = useZodForm({
    schema: schemas.me.validateEmailSchema,
    defaultValues: {
      password: '',
      confirmPassword: '',
    },
  })

  const registerMut = useMutation(resetPassword)

  const onSubmit = async (data: z.infer<typeof schemas.me.validateEmailSchema>) => {
    await registerMut.mutateAsync({
      newPassword: data.password,
      token: token ?? '',
    })
    navigate('/login')
  }
  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-3">
          <InputFormField
            name="password"
            label="Password"
            control={methods.control}
            placeholder="New password"
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
            Reset Password
          </Button>
        </div>
      </form>
    </Form>
  )
}
