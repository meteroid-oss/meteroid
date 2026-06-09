import { createConnectQueryKey, disableQuery, useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import {
  getInviteDetails,
  getInstance,
} from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { initRegistration } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const RETURN_URL_KEY = 'pending_return_url'

export const RegistrationForm = ({ invite }: { invite?: string }) => {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const queryClient = useQueryClient()
  const returnUrl = searchParams.get('returnUrl')

  const { data: inviteData } = useQuery(
    getInviteDetails,
    invite ? { inviteId: invite } : disableQuery,
  )

  const lockedEmail = inviteData?.invitedEmail

  const methods = useZodForm({
    schema: schemas.me.emailSchema,
    defaultValues: {
      email: '',
    },
    mode: 'onSubmit',
  })

  const { setValue } = methods

  useEffect(() => {
    if (lockedEmail) {
      setValue('email', lockedEmail, { shouldValidate: true })
    }
  }, [lockedEmail, setValue])

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

    if (returnUrl && returnUrl.startsWith('/')) {
      sessionStorage.setItem(RETURN_URL_KEY, returnUrl)
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
            autoFocus={!lockedEmail}
            name="email"
            label="Work email"
            control={methods.control}
            placeholder="you@company.com"
            id="signup-email"
            readOnly={!!lockedEmail}
            className={lockedEmail ? 'opacity-50 cursor-not-allowed' : undefined}
            description={lockedEmail ? 'This email is set by your invite and cannot be changed.' : undefined}
          />
          <Button variant="primary" type="submit" disabled={!methods.formState.isValid}>
            Continue
          </Button>
        </div>
      </form>
    </Form>
  )
}
