import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  DialogDescription,
  DialogTitle,
  Form,
  InputFormField,
  Modal,
  Spinner,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Building2, CheckCircle2, CreditCard, ExternalLink, Key } from 'lucide-react'
import { Fragment, KeyboardEvent as ReactKeyboardEvent, createElement, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { ConnectProviderSuccessStep } from '@/features/settings/integrations/ConnectProviderSuccessStep'
import { stripeIntegrationSchema } from '@/features/settings/integrations/schemas'
import { useDismissRouteModal } from '@/hooks/useDismissRouteModal'
import { useTenant } from '@/hooks/useTenant'
import { useZodForm } from '@/hooks/useZodForm'
import { env } from '@/lib/env'
import {
  connectStripe,
  listConnectors,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

export const StripeIntegrationModal = () => {
  const closeModal = useDismissRouteModal()
  const { tenant } = useTenant()

  const restApiUrl = env.meteroidRestApiUri

  const isProduction = tenant?.environment === TenantEnvironmentEnum.PRODUCTION

  const methods = useZodForm({
    mode: 'onChange',
    schema: stripeIntegrationSchema,
    defaultValues: {
      alias: 'stripe',
      apiPublishableKey: '',
      apiSecretKey: '',
      webhookSecret: '',
    },
  })

  const alias = useWatch({
    control: methods.control,
    name: 'alias',
  })

  // Collapsed from 3 steps → 2. The webhook endpoint is now auto-created by
  // the server using the same API key (requires `Webhook Endpoints (write)`
  // scope). Users whose key lacks that scope can paste a webhook secret via
  // the "Advanced" disclosure inside step 2.
  const steps = [
    {
      id: 'alias',
      title: 'Connection',
      description: (
        <>
          Choose a unique alias to identify this connection.
          <br />
          You can connect multiple Stripe accounts.
        </>
      ),
      icon: Building2,
      fields: ['alias'] as const,
    },
    {
      id: 'keys',
      title: 'API Keys',
      description: (
        <span>
          <span>
            Get your {!isProduction && 'test-mode'} API keys from your Stripe Dashboard under{' '}
          </span>
          <br />
          <Button variant="link" hasIcon>
            <ExternalLink size={14} strokeWidth={1.5} />
            <a
              target="_blank"
              href={`https://dashboard.stripe.com/${isProduction ? '' : 'test/'}apikeys`}
              rel="noreferrer"
            >
              Developers → API keys
            </a>
          </Button>
          <br />
          <span className="text-xs text-muted-foreground">
            We&apos;ll create the webhook endpoint for you. The API key needs the&nbsp;
            <code className="text-xs">Webhook Endpoints (write)</code> scope — if yours
            doesn&apos;t, expand <em>Advanced</em> to paste a signing secret manually.
          </span>
        </span>
      ),
      icon: Key,
      // The webhook secret stays in the validated fields list so the
      // (optional) regex check applies when the user fills the Advanced
      // field. Empty is valid — the backend treats empty as "auto-create".
      fields: ['apiPublishableKey', 'apiSecretKey', 'webhookSecret'] as const,
    },
  ]

  const fieldInfo = {
    alias: {
      label: 'Integration Name',
      placeholder: '',
      help: "e.g., 'stripe-eu' ",
    },
    apiPublishableKey: {
      label: 'Publishable Key',
      placeholder: isProduction ? 'pk_live_...' : 'pk_test_...',
      help: undefined,
    },
    apiSecretKey: {
      label: 'Secret Key',
      placeholder: isProduction ? 'sk_live_...' : 'sk_test_...',
      help: undefined,
    },
    webhookSecret: {
      label: 'Webhook Secret',
      placeholder: 'whsec_...',
      help: undefined,
    },
  }

  const [currentStep, setCurrentStep] = useState(0)
  // Set once the connection succeeds — swaps the form for the routing step.
  const [connected, setConnected] = useState<{ id: string; alias: string } | null>(null)

  const queryClient = useQueryClient()
  const connectStripeMutation = useMutation(connectStripe, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listConnectors,
          cardinality: 'finite',
        }),
      })
    },
  })

  const handleNext = () => {
    if (currentStep < steps.length - 1) {
      methods.trigger(steps[currentStep].fields).then(res => {
        if (res) {
          setCurrentStep(prev => prev + 1)
        }
      })
    } else {
      methods.handleSubmit(onSubmit)()
    }
  }

  const handleInputKeyDown = (e: ReactKeyboardEvent<HTMLInputElement>, idx: number) => {
    const { key } = e

    if (key !== 'Enter') {
      return
    }
    e.preventDefault()

    const isLastInput = idx === steps[currentStep].fields.length - 1

    if (isLastInput) {
      handleNext()
    } else {
      methods.setFocus(steps[currentStep].fields[idx + 1])
    }
  }

  const onSubmit = async (data: z.infer<typeof stripeIntegrationSchema>) => {
    // Always send the webhook URL — the backend uses it iff webhookSecret is
    // empty (auto-registration path). When the user pasted a secret in the
    // Advanced section, the backend ignores the URL.
    const autoRegisterWebhookUrl = tenant?.id
      ? `${restApiUrl}/webhooks/v1/${tenant.id}/${data.alias}`
      : undefined

    try {
      const res = await connectStripeMutation.mutateAsync({
        data: {
          alias: data.alias,
          apiPublishableKey: data.apiPublishableKey,
          apiSecretKey: data.apiSecretKey,
          webhookSecret: data.webhookSecret ?? '',
        },
        autoRegisterWebhookUrl,
      })
      toast.success('Connected !')
      // Offer to route payments through the new connector; fall back to closing
      // if the response somehow lacks the connector id.
      if (res.connector?.id) {
        setConnected({ id: res.connector.id, alias: res.connector.alias })
      } else {
        closeModal()
      }
    } catch (error) {
      toast.error(`Failed to connect. API key may be invalid.`)
    }
  }

  const isCurrentStepValid = () => {
    const currentFields = steps[currentStep].fields
    return currentFields.every(field => !methods.formState.errors[field])
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <CreditCard className="w-6 h-6 text-blue" />
            <span>Connect Stripe</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Let&apos;s get your payments flowing in just a few steps
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={true}
      onCancel={() => closeModal()}
      // onConfirm={() => methods.handleSubmit(onSubmit)()}
    >
      <Modal.Content>
        {connected ? (
          <ConnectProviderSuccessStep
            connectorId={connected.id}
            connectorAlias={connected.alias}
            capabilities={{ card: true, directDebit: true }}
            onFinish={() => closeModal()}
          />
        ) : (
          <Form {...methods}>
            <form autoComplete="off">
              <div className="flex items-center justify-center gap-2 mb-6 mt-4">
                {steps.map((_step, idx) => (
                  <Fragment key={idx}>
                    <div
                      className={`flex items-center justify-center w-8 h-8 rounded-full transition-colors ${
                        currentStep === idx
                          ? 'bg-brand text-brand-foreground'
                          : currentStep > idx
                            ? 'bg-success text-success-foreground'
                            : 'bg-muted text-muted-foreground'
                      }`}
                    >
                      {currentStep > idx ? <CheckCircle2 className="w-5 h-5" /> : idx + 1}
                    </div>
                    {idx < steps.length - 1 && (
                      <div
                        className={`h-0.5 w-16 transition-colors ${
                          currentStep > idx ? 'bg-success' : 'bg-gray-200'
                        }`}
                      />
                    )}
                  </Fragment>
                ))}
              </div>

              {/* Current step icon */}
              <div className="flex justify-center">
                {createElement(steps[currentStep].icon, {
                  className: 'w-12 h-12 text-brand',
                  strokeWidth: 1.2,
                })}
              </div>

              <div className="text-center space-y-2 mb-6 mt-2">
                <h3 className="text-md font-semibold">{steps[currentStep].title}</h3>
                <p className="text-muted-foreground text-sm">{steps[currentStep].description}</p>
              </div>

              <div className="space-y-6">
                {/* Render the non-webhook fields inline; webhookSecret lives
                  inside the Advanced disclosure below so the happy path is
                  just (alias + 2 keys) on step 2. */}
                {steps[currentStep].fields
                  .filter(field => field !== 'webhookSecret')
                  .map((field, idx) => (
                    <div key={field} className="space-y-2">
                      <InputFormField
                        control={methods.control}
                        label={fieldInfo[field].label}
                        name={field}
                        layout="vertical"
                        description={fieldInfo[field].help}
                        placeholder={fieldInfo[field].placeholder}
                        showPasswordToggle={field === 'apiSecretKey'}
                        data-form-type="other"
                        onKeyDown={ev => handleInputKeyDown(ev, idx)}
                      />
                    </div>
                  ))}

                {/* Advanced disclosure on step 2: lets users with restricted
                  API keys paste a manually-created webhook secret instead of
                  relying on auto-registration. */}
                {(steps[currentStep].fields as readonly string[]).includes('webhookSecret') && (
                  <details className="rounded-md border border-border bg-card px-3 py-2">
                    <summary className="cursor-pointer text-xs text-muted-foreground">
                      Advanced: my API key can&apos;t create webhook endpoints
                    </summary>
                    <div className="pt-3 space-y-2">
                      <p className="text-xs text-muted-foreground">
                        Create a webhook in your Stripe dashboard pointing at this URL, then paste
                        its signing secret here.
                      </p>
                      <CopyToClipboardButton
                        text={`${restApiUrl}/webhooks/v1/${tenant?.id}/${alias}`}
                        className="whitespace-normal"
                      />
                      <InputFormField
                        control={methods.control}
                        label={fieldInfo.webhookSecret.label}
                        name="webhookSecret"
                        layout="vertical"
                        placeholder={fieldInfo.webhookSecret.placeholder}
                        showPasswordToggle
                        data-form-type="other"
                      />
                    </div>
                  </details>
                )}

                <div className="flex justify-end gap-2 py-3 px-5 border-t ">
                  <div className="flex w-full space-x-2 justify-end">
                    <Button
                      variant="secondary"
                      onClick={() =>
                        currentStep > 0 ? setCurrentStep(prev => prev - 1) : closeModal()
                      }
                      type="button"
                      size="sm"
                    >
                      {currentStep > 0 ? 'Back' : 'Cancel'}
                    </Button>
                    <Button
                      type="button"
                      onClick={handleNext}
                      hasIcon={connectStripeMutation.isPending}
                      size="sm"
                      disabled={!isCurrentStepValid() || methods.formState.isSubmitting}
                    >
                      {connectStripeMutation.isPending && <Spinner />}
                      {currentStep === steps.length - 1 ? 'Connect Stripe' : 'Continue'}
                    </Button>
                  </div>
                </div>
              </div>
            </form>
          </Form>
        )}
      </Modal.Content>
    </Modal>
  )
}
