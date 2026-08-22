import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, DialogDescription, DialogTitle, Form, InputFormField, Modal, Spinner } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CreditCard, ExternalLink } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { ConnectProviderSuccessStep } from '@/features/settings/integrations/ConnectProviderSuccessStep'
import { stancerIntegrationSchema } from '@/features/settings/integrations/schemas'
import { useDismissRouteModal } from '@/hooks/useDismissRouteModal'
import { useZodForm } from '@/hooks/useZodForm'
import {
  connectStancer,
  listConnectors,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'

/**
 * Connect a Stancer merchant account.
 *
 * Single step, unlike Stripe/GoCardless: the secret key is the only credential.
 * Its prefix selects the environment (`stest_` test / `sprod_` live) — there is
 * no environment toggle — and Stancer has no webhooks, so no endpoint setup.
 * The backend validates the key against the Stancer API before persisting.
 */
export const StancerIntegrationModal = () => {
  const closeModal = useDismissRouteModal()

  const methods = useZodForm({
    mode: 'onChange',
    schema: stancerIntegrationSchema,
    defaultValues: {
      alias: 'stancer',
      apiSecretKey: '',
    },
  })

  const [showCloseConfirm, setShowCloseConfirm] = useState(false)
  // Set once the connection succeeds — swaps the form for the routing step.
  const [connected, setConnected] = useState<{ id: string; alias: string } | null>(null)

  const queryClient = useQueryClient()
  const connectStancerMutation = useMutation(connectStancer, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listConnectors,
          cardinality: undefined,
        }),
      })
    },
  })

  const onSubmit = async (data: z.infer<typeof stancerIntegrationSchema>) => {
    try {
      const res = await connectStancerMutation.mutateAsync({
        data: {
          alias: data.alias,
          apiSecretKey: data.apiSecretKey,
        },
      })
      toast.success('Connected!')
      // Offer to route card payments through the new connector; fall back to
      // closing if the response somehow lacks the connector id.
      if (res.connector?.id) {
        setConnected({ id: res.connector.id, alias: res.connector.alias })
      } else {
        closeModal()
      }
    } catch (error) {
      toast.error('Failed to connect. Secret key may be invalid.')
    }
  }

  return (
    <>
      <Modal
        header={
          <>
            <DialogTitle className="flex items-center gap-2 text-md">
              <CreditCard className="w-6 h-6 text-blue" />
              <span>Connect Stancer</span>
            </DialogTitle>
            <DialogDescription className="text-sm">
              Set up card collection with your Stancer account
            </DialogDescription>
          </>
        }
        visible={true}
        hideFooter={true}
        // Once connected there's nothing unsaved to lose — closing just leaves.
        onCancel={() => (connected ? closeModal() : setShowCloseConfirm(true))}
        onInteractOutside={e => {
          e.preventDefault()
          if (connected) closeModal()
          else setShowCloseConfirm(true)
        }}
        onEscapeKeyDown={e => {
          e.preventDefault()
          if (connected) closeModal()
          else setShowCloseConfirm(true)
        }}
      >
        <Modal.Content>
          {connected ? (
            <ConnectProviderSuccessStep
              connectorId={connected.id}
              connectorAlias={connected.alias}
              capabilities={{ card: true, directDebit: false }}
              onFinish={closeModal}
            />
          ) : (
            <Form {...methods}>
              <form autoComplete="off" onSubmit={methods.handleSubmit(onSubmit)}>
                <div className="text-center space-y-2 mb-6 mt-2">
                  <p className="text-muted-foreground text-sm">
                    <span>Find your secret key in your Stancer dashboard under </span>
                    <Button variant="link" hasIcon>
                      <ExternalLink size={14} strokeWidth={1.5} />
                      <a
                        target="_blank"
                        href="https://manage.stancer.com/en/developers"
                        rel="noreferrer"
                      >
                        Developers → API keys
                      </a>
                    </Button>
                    <br />
                    <span className="text-xs text-muted-foreground">
                      A test key (stest_...) connects in test mode, a live key (sprod_...) in live
                      mode.
                    </span>
                  </p>
                </div>

                <div className="space-y-6">
                  <InputFormField
                    control={methods.control}
                    label="Integration Name"
                    name="alias"
                    layout="vertical"
                    description="e.g., 'stancer-eu'"
                    data-form-type="other"
                  />
                  <InputFormField
                    control={methods.control}
                    label="Secret Key"
                    name="apiSecretKey"
                    layout="vertical"
                    placeholder="stest_..."
                    showPasswordToggle
                    data-form-type="other"
                  />

                  <div className="flex justify-end gap-2 py-3 px-5 border-t">
                    <div className="flex w-full space-x-2 justify-end">
                      <Button
                        variant="secondary"
                        onClick={() => setShowCloseConfirm(true)}
                        type="button"
                        size="sm"
                      >
                        Cancel
                      </Button>
                      <Button
                        type="submit"
                        hasIcon={connectStancerMutation.isPending}
                        size="sm"
                        disabled={!methods.formState.isValid || methods.formState.isSubmitting}
                      >
                        {connectStancerMutation.isPending && <Spinner />}
                        Connect Stancer
                      </Button>
                    </div>
                  </div>
                </div>
              </form>
            </Form>
          )}
        </Modal.Content>
      </Modal>

      <Modal
        visible={showCloseConfirm}
        size="small"
        header="Discard connection setup?"
        confirmText="Discard"
        cancelText="Keep editing"
        onCancel={() => setShowCloseConfirm(false)}
        onConfirm={closeModal}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-muted-foreground">
            This Stancer connection hasn&apos;t been saved yet. If you close now, the details you
            entered will be lost.
          </p>
        </Modal.Content>
      </Modal>
    </>
  )
}
