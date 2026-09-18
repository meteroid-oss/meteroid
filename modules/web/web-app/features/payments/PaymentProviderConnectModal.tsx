import { Code, ConnectError } from '@connectrpc/connect'
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
import { CheckCircle2 } from 'lucide-react'
import {
  Fragment,
  KeyboardEvent as ReactKeyboardEvent,
  ReactNode,
  cloneElement,
  createElement,
  isValidElement,
  useEffect,
  useState,
} from 'react'
import { Control, useWatch } from 'react-hook-form'
import { useParams } from 'react-router-dom'
import { toast } from 'sonner'

import { ConnectProviderSuccessStep } from '@/features/settings/integrations/ConnectProviderSuccessStep'
import { useDismissRouteModal } from '@/hooks/useDismissRouteModal'
import { useTenant } from '@/hooks/useTenant'
import { useZodForm } from '@/hooks/useZodForm'
import { env } from '@/lib/env'
import {
  connectPaymentProvider,
  listConnectors,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

import {
  ConnectContext,
  ConnectField,
  ConnectFieldSlot,
  PaymentProviderDefinition,
  paymentProviderByKey,
  railsOf,
} from './providers'

type FormValues = Record<string, string | undefined>

/** Route modal at `connect-payment-provider/:providerKey`, driven entirely by the registry. */
export const PaymentProviderConnectModal = () => {
  const { providerKey } = useParams<{ providerKey: string }>()
  const closeModal = useDismissRouteModal()
  const definition = paymentProviderByKey(providerKey)

  useEffect(() => {
    if (!definition) closeModal()
  }, [definition, closeModal])

  if (!definition) return null
  return <ConnectModal definition={definition} />
}

const ConnectModal = ({ definition }: { definition: PaymentProviderDefinition }) => {
  const { connect, name, Logo } = definition
  const closeModal = useDismissRouteModal()
  const { tenant } = useTenant()

  const methods = useZodForm({
    mode: 'onChange',
    schema: connect.schema,
    defaultValues: connect.defaults,
  })
  const control = methods.control as unknown as Control<FormValues>
  const alias = useWatch({ control, name: 'alias' }) ?? ''

  const ctx: ConnectContext = {
    tenantId: tenant?.id,
    isProduction: tenant?.environment === TenantEnvironmentEnum.PRODUCTION,
    restApiUrl: env.meteroidRestApiUri,
    alias,
  }

  const [currentStep, setCurrentStep] = useState(0)
  const [showCloseConfirm, setShowCloseConfirm] = useState(false)
  // Set once connected: replaces the form with the routing step.
  const [connected, setConnected] = useState<{
    id: string
    alias: string
    rails: ReturnType<typeof railsOf>
  } | null>(null)

  const queryClient = useQueryClient()
  const connectMutation = useMutation(connectPaymentProvider, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listConnectors, cardinality: undefined }),
      })
    },
  })

  const steps = connect.steps
  const step = steps[currentStep]
  const isLast = currentStep === steps.length - 1

  const onSubmit = async (values: FormValues) => {
    try {
      const res = await connectMutation.mutateAsync({
        credentials: connect.toCredentials(values, ctx),
        autoRegisterWebhookUrl:
          connect.autoRegisterWebhook && ctx.tenantId
            ? `${ctx.restApiUrl}/webhooks/v1/${ctx.tenantId}/${values.alias}`
            : undefined,
      })
      toast.success('Connected!')
      if (res.connector?.id) {
        setConnected({
          id: res.connector.id,
          alias: res.connector.alias,
          rails: railsOf(res.connector.paymentCapabilities),
        })
      } else {
        closeModal()
      }
    } catch (error) {
      // Only validation errors (e.g. a rejected key) are user-facing.
      const err = ConnectError.from(error)
      toast.error(
        err.code === Code.InvalidArgument && err.rawMessage ? err.rawMessage : connect.errorFallback
      )
    }
  }

  const handleNext = () => {
    if (!isLast) {
      methods.trigger([...step.fields]).then(ok => ok && setCurrentStep(prev => prev + 1))
    } else {
      methods.handleSubmit(onSubmit)()
    }
  }

  const handleInputKeyDown = (e: ReactKeyboardEvent<HTMLInputElement>, idx: number) => {
    if (e.key !== 'Enter') return
    e.preventDefault()
    if (idx === step.fields.length - 1) handleNext()
    else methods.setFocus(step.fields[idx + 1])
  }

  const renderField = (fieldName: string, idx?: number) => {
    const field: ConnectField = connect.fields[fieldName]
    const placeholder =
      typeof field.placeholder === 'function' ? field.placeholder(ctx) : field.placeholder
    return (
      <InputFormField
        control={control}
        label={field.label}
        name={fieldName}
        layout="vertical"
        description={field.help}
        placeholder={placeholder}
        showPasswordToggle={field.secret}
        data-form-type="other"
        onKeyDown={idx === undefined ? undefined : ev => handleInputKeyDown(ev, idx)}
      />
    )
  }

  const isStepValid = step.fields.every(field => !methods.formState.errors[field])
  const dismiss = () => (connected ? closeModal() : setShowCloseConfirm(true))

  return (
    <>
      <Modal
        header={
          <>
            <DialogTitle className="flex items-center gap-2 text-md">
              <Logo className="w-6 h-6 text-foreground" />
              <span>Connect {name}</span>
            </DialogTitle>
            <DialogDescription className="text-sm">{connect.subtitle}</DialogDescription>
          </>
        }
        visible={true}
        hideFooter={true}
        onCancel={dismiss}
        onInteractOutside={e => {
          e.preventDefault()
          // Close on click only: on a direct link the Settings dialog behind can steal focus.
          if (e.type === 'dismissableLayer.focusOutside') return
          dismiss()
        }}
        onEscapeKeyDown={e => {
          e.preventDefault()
          dismiss()
        }}
      >
        <Modal.Content>
          {connected ? (
            <ConnectProviderSuccessStep
              connectorId={connected.id}
              connectorAlias={connected.alias}
              capabilities={connected.rails}
              onFinish={closeModal}
            />
          ) : (
            <Form {...methods}>
              <form autoComplete="off">
                {steps.length > 1 && (
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
                )}

                <div className="flex justify-center mt-4">
                  {createElement(step.icon, {
                    className: 'w-12 h-12 text-brand',
                    strokeWidth: 1.2,
                  })}
                </div>

                <div className="text-center space-y-2 mb-6 mt-2">
                  <h3 className="text-md font-semibold">{step.title}</h3>
                  <p className="text-muted-foreground text-sm">{step.description(ctx)}</p>
                </div>

                <div className="space-y-6">
                  {step.fields.map((fieldName, idx) => (
                    <div key={fieldName} className="space-y-2">
                      {renderField(fieldName, idx)}
                    </div>
                  ))}

                  {step.extra && bindFieldSlots(step.extra(ctx), renderField)}

                  <div className="flex justify-end gap-2 py-3 px-5 border-t">
                    <div className="flex w-full space-x-2 justify-end">
                      <Button
                        variant="secondary"
                        onClick={() =>
                          currentStep > 0
                            ? setCurrentStep(prev => prev - 1)
                            : setShowCloseConfirm(true)
                        }
                        type="button"
                        size="sm"
                      >
                        {currentStep > 0 ? 'Back' : 'Cancel'}
                      </Button>
                      <Button
                        type="button"
                        onClick={handleNext}
                        hasIcon={connectMutation.isPending}
                        size="sm"
                        disabled={!isStepValid || methods.formState.isSubmitting}
                      >
                        {connectMutation.isPending && <Spinner />}
                        {isLast ? `Connect ${name}` : 'Continue'}
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
            This {name} connection hasn&apos;t been saved yet. If you close now, the details you
            entered will be lost.
          </p>
        </Modal.Content>
      </Modal>
    </>
  )
}

/** Replaces every `ConnectFieldSlot` in a step's extra block with the bound input. */
const bindFieldSlots = (node: ReactNode, renderField: (name: string) => ReactNode): ReactNode => {
  if (Array.isArray(node)) {
    return node.map((child, i) => <Fragment key={i}>{bindFieldSlots(child, renderField)}</Fragment>)
  }
  if (!isValidElement(node)) return node
  if (node.type === ConnectFieldSlot) {
    return renderField((node.props as { name: string }).name)
  }
  const props = node.props as { children?: ReactNode }
  if (props.children === undefined) return node
  return cloneElement(node, undefined, bindFieldSlots(props.children, renderField))
}
