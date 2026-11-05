import { Code, ConnectError } from '@connectrpc/connect'
import { Button, Form, InputFormField } from '@md/ui'
import { useMutation } from '@tanstack/react-query'
import { Clock, MailCheck } from 'lucide-react'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { env } from '@/lib/env'
import { schemas } from '@/lib/schemas'

// Must match `PORTAL_LINK_EXPIRED` in the backend.
const PORTAL_LINK_EXPIRED = 'Portal link expired'

export const isPortalLinkExpired = (error: unknown) =>
  error instanceof ConnectError &&
  error.code === Code.Unauthenticated &&
  error.rawMessage === PORTAL_LINK_EXPIRED

const requestNewLink = async (email: string) => {
  const token = new URLSearchParams(window.location.search).get('token') ?? ''
  const res = await fetch(`${env.meteroidRestApiUri.replace(/\/$/, '')}/v1/portal/links/recover`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ token, email }),
  })
  if (!res.ok) {
    throw new Error('The request could not be sent, please try again later.')
  }
}

export const LinkExpired = ({ document }: { document: 'invoice' | 'quote' }) => {
  const methods = useZodForm({
    schema: schemas.me.emailSchema,
    defaultValues: { email: '' },
  })
  const recover = useMutation({ mutationFn: requestNewLink })

  const onSubmit = (data: z.infer<typeof schemas.me.emailSchema>) =>
    recover.mutate(data.email, {
      onError: err => methods.setError('email', { message: err.message }),
    })

  if (recover.isSuccess) {
    return (
      <div className="flex flex-col items-center text-center max-w-md mx-auto px-6 py-12">
        <MailCheck className="h-8 w-8 text-muted-foreground mb-4" />
        <h2 className="text-md font-semibold text-gray-800 mb-2">Check your inbox</h2>
        <p className="text-gray-800 text-sm">
          If this address received the original email, a new link to this {document} is on its way.
        </p>
      </div>
    )
  }

  return (
    <div className="flex flex-col items-center text-center max-w-md mx-auto px-6 py-12">
      <Clock className="h-8 w-8 text-muted-foreground mb-4" />
      <h2 className="text-md font-semibold text-gray-800 mb-2">This link has expired</h2>
      <p className="text-gray-800 text-sm mb-6">
        Enter the email address this {document} was sent to and we&apos;ll email you a new link.
      </p>
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="w-full text-left">
          <div className="flex flex-col gap-2">
            <InputFormField
              autoFocus
              name="email"
              label="Email address"
              control={methods.control}
              placeholder="you@company.com"
              id="portal-link-recovery"
            />
            <Button
              variant="primary"
              type="submit"
              className="mt-3"
              disabled={!methods.formState.isValid || recover.isPending}
            >
              Send me a new link
            </Button>
          </div>
        </form>
      </Form>
    </div>
  )
}
