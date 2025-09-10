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
import { Building2, CheckCircle2, CreditCard, ExternalLink, Key, WebhookIcon } from 'lucide-react'
import { Fragment, KeyboardEvent as ReactKeyboardEvent, createElement, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { stripeIntegrationSchema } from '@/features/settings/integrations/schemas'
import { useTenant } from '@/hooks/useTenant'
import { useZodForm } from '@/hooks/useZodForm'
import { env } from '@/lib/env'
import {
  connectStripe,
  listConnectors,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

export const StripeIntegrationModal = () => {
  const navigate = useNavigate()
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

  const steps = [
    {
      id: 'alias',
      title: 'Connection',
      description: (
        <>
          Choose a unique alias to identify this connection.
          <br/>
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
          <br/>
          <Button variant="link" hasIcon>
            <ExternalLink size={14} strokeWidth={1.5}/>
            <a
              target="_blank"
              href={`https://dashboard.stripe.com/${isProduction ? '' : 'test/'}apikeys`}
              rel="noreferrer"
            >
              Developers → API keys
            </a>
          </Button>
          <br/>
        </span>
      ),
      icon: Key,
      fields: ['apiPublishableKey', 'apiSecretKey'] as const,
    },
    {
      id: 'webhook',
      title: 'Webhooks',
      description: (
        <span>
          <span>Create a webhook endpoint in your Stripe Dashboard under </span>
          <br/>
          <Button variant="link" hasIcon>
            <ExternalLink size={14} strokeWidth={1.5}/>
            <a
              target="_blank"
              href={`https://dashboard.stripe.com/${isProduction ? '' : 'test/'}webhooks/create?events=setup_intent.succeeded%2Cpayment_intent.succeeded%2Cpayment_intent.partially_funded%2Cpayment_intent.payment_failed&url=${restApiUrl}/v1/webhooks/${tenant?.id?.toLowerCase()}/${alias}`}
              rel="noreferrer"
            >
              Developers → Webhooks
            </a>
          </Button>
          <br/>

          <div className="bg-card p-4 rounded-lg space-y-3 mt-4  ">
            <ol className="space-y-2 text-sm text-card-foreground">
              <li>
                Endpoint URL:
                <br/>
                <CopyToClipboardButton
                  text={`${restApiUrl}/webhooks/v1/${tenant?.id}/${alias}`}
                  className="whitespace-normal"
                />
              </li>

              <li>
                The following events should be selected : <br/>
                <div className="font-mono text-xs bg-background dark:bg-secondary rounded-md py-2">
                  <code>
                    payment_intent.succeeded, payment_intent.payment_failed,
                    payment_intent.partially_funded
                  </code>
                </div>
                <br/>
              </li>
              <li>
                {`Then click on "Add an endpoint", reveal and copy the "Signing secret" to the form
                input below`}
              </li>
            </ol>
          </div>
        </span>
      ),
      icon: WebhookIcon,
      fields: ['webhookSecret'] as const,
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

  const queryClient = useQueryClient()
  const connectStripeMutation = useMutation(connectStripe, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listConnectors),
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
    try {
      await connectStripeMutation.mutateAsync({
        data: {
          alias: data.alias,
          apiPublishableKey: data.apiPublishableKey,
          apiSecretKey: data.apiSecretKey,
          webhookSecret: data.webhookSecret,
        },
      })
      toast.success('Connected !')
      navigate('..')
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
            <CreditCard className="w-6 h-6 text-blue"/>
            <span>Connect Stripe</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Let&apos;s get your payments flowing in just a few steps
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={true}
      onCancel={() => navigate('..')}
      // onConfirm={() => methods.handleSubmit(onSubmit)()}
    >
      <Modal.Content>
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
                    {currentStep > idx ? <CheckCircle2 className="w-5 h-5"/> : idx + 1}
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
              {/* With react-hook-form, you'd use form.register instead of the onChange handler */}
              {steps[currentStep].fields.map((field, idx) => (
                <div key={field} className="space-y-2">
                  <InputFormField
                    control={methods.control}
                    label={fieldInfo[field].label}
                    name={field}
                    layout="vertical"
                    description={fieldInfo[field].help}
                    placeholder={fieldInfo[field].placeholder}
                    showPasswordToggle={['apiSecretKey', 'webhookSecret'].includes(field)}
                    data-form-type="other"
                    onKeyDown={ev => handleInputKeyDown(ev, idx)}
                  />
                </div>
              ))}

              <div className="flex justify-end gap-2 py-3 px-5 border-t ">
                <div className="flex w-full space-x-2 justify-end">
                  <Button
                    variant="secondary"
                    onClick={() =>
                      currentStep > 0 ? setCurrentStep(prev => prev - 1) : navigate('..')
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
                    {connectStripeMutation.isPending && <Spinner/>}
                    {currentStep === steps.length - 1 ? 'Connect Stripe' : 'Continue'}
                  </Button>
                </div>
              </div>
            </div>
          </form>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
