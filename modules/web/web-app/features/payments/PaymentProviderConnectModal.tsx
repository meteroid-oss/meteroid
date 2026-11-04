import { useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  Button,
  DialogDescription,
  DialogTitle,
  Form,
  Input,
  InputFormField,
  Label,
  Modal,
  Spinner,
  cn,
} from '@md/ui'
import { Check, CheckCircle2, X } from 'lucide-react'
import { Fragment, useEffect, useMemo, useState } from 'react'
import { Control } from 'react-hook-form'
import { useParams, useSearchParams } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { useDismissRouteModal } from '@/hooks/useDismissRouteModal'
import { useTenant } from '@/hooks/useTenant'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  createPaymentProviderDraft,
  deleteConnector,
  listConnectors,
  renameConnector,
  updatePaymentProviderCredentials,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { Connector, ConnectorStatusEnum } from '@/rpc/api/connectors/v1/models_pb'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

import { AccountDetails } from './AccountDetails'
import { ConnectReviewStep } from './ConnectReviewStep'
import { useInvalidateConnectors } from './ConnectorLifecycle'
import { CredentialFields, CredentialValues } from './CredentialFields'
import { WizardFooter } from './WizardFooter'
import {
  MethodChips,
  wizardSteps,
  WizardStep,
  methodChips,
  resumeStep,
  userFacingError,
} from './connectors'
import {
  ConnectContext,
  PaymentProviderDefinition,
  aliasSchema,
  credentialsSchema,
  paymentProviderByKey,
} from './providers'

/**
 * Route modal at `connect-payment-provider/:providerKey[?draft=<id>]`, driven by the registry.
 * Step 1 creates the connector as a draft, so later steps save on the server and closing loses
 * nothing; `?draft=` resumes at the first unfinished step.
 */
export const PaymentProviderConnectModal = () => {
  const { providerKey } = useParams<{ providerKey: string }>()
  const [searchParams] = useSearchParams()
  const closeModal = useDismissRouteModal()
  const definition = paymentProviderByKey(providerKey)
  const draftId = searchParams.get('draft') ?? undefined

  useEffect(() => {
    if (!definition) closeModal()
  }, [definition, closeModal])

  if (!definition) return null
  return draftId ? (
    <ResumeDraft definition={definition} draftId={draftId} />
  ) : (
    <ConnectWizard definition={definition} />
  )
}

const ResumeDraft = ({
  definition,
  draftId,
}: {
  definition: PaymentProviderDefinition
  draftId: string
}) => {
  const closeModal = useDismissRouteModal()
  const connectorsQuery = useQuery(listConnectors, {})
  // Latched: the wizard's own saves refetch the list (and activation ends the draft status).
  const [draft, setDraft] = useState<Connector>()

  useEffect(() => {
    if (draft || !connectorsQuery.isSuccess) return
    const found = connectorsQuery.data.connectors.find(c => c.id === draftId)
    if (found?.status === ConnectorStatusEnum.DRAFT) {
      setDraft(found)
      return
    }
    toast.error(found ? `${found.alias} is already set up.` : 'This draft no longer exists.')
    closeModal()
  }, [draft, connectorsQuery.isSuccess, connectorsQuery.data, draftId, closeModal])

  return draft ? <ConnectWizard definition={definition} initialDraft={draft} /> : null
}

type ClosePrompt = 'discard' | 'finishLater'

const ConnectWizard = ({
  definition,
  initialDraft,
}: {
  definition: PaymentProviderDefinition
  initialDraft?: Connector
}) => {
  const { connect, name, Logo } = definition
  const closeModal = useDismissRouteModal()
  const { tenant } = useTenant()
  const invalidate = useInvalidateConnectors()

  const [connector, setConnector] = useState<Connector | undefined>(initialDraft)
  const [step, setStep] = useState<WizardStep>(
    initialDraft ? resumeStep(initialDraft, definition) : 'name'
  )
  const [closePrompt, setClosePrompt] = useState<ClosePrompt>()

  const ctx: ConnectContext = {
    isProduction: tenant?.environment === TenantEnvironmentEnum.PRODUCTION,
    webhookUrl: connector?.webhookUrl,
  }

  const onSaved = (updated: Connector | undefined) => {
    if (updated) setConnector(updated)
    invalidate()
  }

  const goBack = () =>
    setStep(prev => {
      if (prev === 'review') return connect.webhook ? 'webhook' : 'credentials'
      if (prev === 'webhook') return 'credentials'
      return 'name'
    })

  const dismiss = () => {
    if (connector) setClosePrompt('finishLater')
    else setClosePrompt('discard')
  }

  const subtitle = !connector
    ? connect.subtitle
    : step === 'review'
      ? `${connector.alias} · ready to activate`
      : `${connector.alias} · draft saved`

  return (
    <>
      <Modal
        header={
          <div className="flex items-start justify-between gap-3">
            <div className="space-y-1">
              <DialogTitle className="flex items-center gap-2 text-md">
                <Logo className="w-6 h-6 text-foreground" />
                <span>Connect {name}</span>
              </DialogTitle>
              <DialogDescription className="text-sm">{subtitle}</DialogDescription>
            </div>
            {/* Asks to finish later or discard the draft, like Escape and clicking outside. */}
            <Button size="icon" variant="ghost" title="Close" aria-label="Close" onClick={dismiss}>
              <X size={16} />
            </Button>
          </div>
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
          <Stepper current={step} steps={wizardSteps(definition)} />
          {step === 'name' && (
            <NameStep
              definition={definition}
              ctx={ctx}
              connector={connector}
              onCancel={dismiss}
              onSaved={updated => {
                onSaved(updated)
                setStep('credentials')
              }}
            />
          )}
          {step === 'credentials' && connector && (
            <CredentialsStep
              definition={definition}
              ctx={ctx}
              connector={connector}
              onBack={goBack}
              onSaved={onSaved}
              onContinue={() => setStep(connect.webhook ? 'webhook' : 'review')}
            />
          )}
          {step === 'webhook' && connector && connect.webhook && (
            <WebhookStep
              definition={definition}
              ctx={ctx}
              connector={connector}
              onBack={goBack}
              onSaved={onSaved}
              onContinue={() => setStep('review')}
            />
          )}
          {step === 'review' && connector && (
            <ConnectReviewStep
              connector={connector}
              definition={definition}
              webhookNote={
                connect.webhook
                  ? connector.hasWebhookSecret
                    ? 'Configured'
                    : 'Not configured'
                  : undefined
              }
              onBack={goBack}
              onDone={closeModal}
            />
          )}
        </Modal.Content>
      </Modal>

      <Modal
        visible={closePrompt === 'discard'}
        size="small"
        header="Discard connection setup?"
        confirmText="Discard"
        cancelText="Keep editing"
        onCancel={() => setClosePrompt(undefined)}
        onConfirm={closeModal}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-muted-foreground">
            This {name} connection hasn&apos;t been saved yet. If you close now, the name you
            entered will be lost.
          </p>
        </Modal.Content>
      </Modal>

      {connector && (
        <FinishLaterPrompt
          visible={closePrompt === 'finishLater'}
          connector={connector}
          onKeepEditing={() => setClosePrompt(undefined)}
          onClose={closeModal}
        />
      )}
    </>
  )
}

const Stepper = ({
  current,
  steps,
}: {
  current: WizardStep
  steps: { id: WizardStep; title: string }[]
}) => {
  const currentIdx = steps.findIndex(s => s.id === current)
  return (
    <div className="mb-6 mt-4 flex items-center justify-center gap-2">
      {steps.map((s, idx) => {
        const done = idx < currentIdx
        return (
          <Fragment key={s.id}>
            <div className="flex flex-col items-center gap-1">
              <div
                className={cn(
                  'flex h-7 w-7 items-center justify-center rounded-full text-xs transition-colors',
                  idx === currentIdx
                    ? 'bg-brand text-brand-foreground'
                    : done
                      ? 'bg-success text-success-foreground'
                      : 'bg-muted text-muted-foreground'
                )}
              >
                {done ? <Check className="h-4 w-4" /> : idx + 1}
              </div>
              <span
                className={cn(
                  'text-[11px]',
                  idx === currentIdx ? 'text-foreground' : 'text-muted-foreground'
                )}
              >
                {s.title}
              </span>
            </div>
            {idx < steps.length - 1 && (
              <div
                className={cn(
                  'mb-4 h-0.5 w-10 transition-colors',
                  idx < currentIdx ? 'bg-success' : 'bg-muted'
                )}
              />
            )}
          </Fragment>
        )
      })}
    </div>
  )
}

interface StepProps {
  definition: PaymentProviderDefinition
  ctx: ConnectContext
  connector: Connector
  onBack: () => void
  onSaved: (connector: Connector | undefined) => void
  onContinue: () => void
}

const aliasFormSchema = z.object({ alias: aliasSchema })

const NameStep = ({
  definition,
  ctx,
  connector,
  onCancel,
  onSaved,
}: {
  definition: PaymentProviderDefinition
  ctx: ConnectContext
  connector?: Connector
  onCancel: () => void
  onSaved: (connector: Connector | undefined) => void
}) => {
  const { connect } = definition
  const methods = useZodForm({
    mode: 'onChange',
    schema: aliasFormSchema,
    defaultValues: { alias: connector?.alias ?? connect.aliasDefault },
  })
  const createMut = useMutation(createPaymentProviderDraft)
  const renameMut = useMutation(renameConnector)

  const onSubmit = async ({ alias }: z.infer<typeof aliasFormSchema>) => {
    try {
      if (!connector) {
        const res = await createMut.mutateAsync({ provider: definition.provider, alias })
        onSaved(res.connector)
      } else if (alias !== connector.alias) {
        const res = await renameMut.mutateAsync({ id: connector.id, alias })
        onSaved(res.connector)
      } else {
        onSaved(connector)
      }
    } catch (error) {
      // Mostly "name already in use", which belongs next to the field.
      methods.setError('alias', {
        message: userFacingError(error, "Couldn't save the connection. Please try again."),
      })
    }
  }

  const pending = createMut.isPending || renameMut.isPending

  return (
    <Form {...methods}>
      <form autoComplete="off" onSubmit={methods.handleSubmit(onSubmit)} className="space-y-5">
        <InputFormField
          control={methods.control}
          name="alias"
          label="Integration name"
          layout="vertical"
          description={`Lowercase letters, numbers and hyphens, e.g. '${connect.aliasExample}'. You can rename it later.`}
          data-form-type="other"
          autoFocus
        />
        {/* The environment follows the tenant (live in production, sandbox otherwise); not user-selectable. */}
        {connect.showsEnvironment && (
          <div className="space-y-2">
            <Label className="text-xs font-normal dark:text-muted-foreground">Environment</Label>
            <div className="rounded-md border border-border bg-muted px-3 py-2 text-sm">
              {ctx.isProduction ? 'Live' : 'Sandbox'}
            </div>
            <p className="text-xs text-muted-foreground">
              Set by this tenant: live in production, sandbox otherwise.
            </p>
          </div>
        )}
        <WizardFooter>
          <Button variant="secondary" size="sm" type="button" onClick={onCancel}>
            Cancel
          </Button>
          <Button size="sm" type="submit" hasIcon={pending} disabled={pending}>
            {pending && <Spinner />}
            Continue
          </Button>
        </WizardFooter>
      </form>
    </Form>
  )
}

const CredentialsStep = ({
  definition,
  ctx,
  connector,
  onBack,
  onSaved,
  onContinue,
}: StepProps) => {
  const { connect, name } = definition
  // Verified credentials show the detected account first, so the user can confirm it.
  const [editing, setEditing] = useState(!connector.hasCredentials)
  const [error, setError] = useState<string>()

  const schema = useMemo(
    () => credentialsSchema(connect, connect.credentialFields, 'create'),
    [connect]
  )
  const methods = useZodForm({
    mode: 'onChange',
    schema,
    defaultValues: Object.fromEntries(connect.credentialFields.map(f => [f, ''])),
  })
  const updateMut = useMutation(updatePaymentProviderCredentials)

  const onSubmit = async (values: CredentialValues) => {
    setError(undefined)
    try {
      const res = await updateMut.mutateAsync({
        id: connector.id,
        credentials: connect.toCredentials(values, ctx),
      })
      onSaved(res.connector)
      methods.reset()
      setEditing(false)
    } catch (e) {
      // Shown inline and the fields kept as entered, so a typo is a quick fix.
      setError(userFacingError(e, connect.errorFallback, name))
    }
  }
  const submit = methods.handleSubmit(onSubmit)

  if (!editing) {
    const account = connector.account
    return (
      <div className="space-y-5">
        <div className="space-y-4 rounded-lg border border-success/40 bg-success/5 p-4">
          <p className="flex items-center gap-2 text-sm font-medium">
            <CheckCircle2 className="h-4 w-4 text-success" />
            Connected{account?.displayName ? ` to ${account.displayName}` : ` to ${name}`}
          </p>
          <AccountDetails connector={connector} definition={definition} />
          <div className="space-y-2">
            <p className="text-xs font-medium">Payment methods on this account</p>
            <MethodChips chips={methodChips(connector)} />
          </div>
        </div>
        <WizardFooter>
          <Button variant="secondary" size="sm" type="button" onClick={() => setEditing(true)}>
            Change credentials
          </Button>
          <Button size="sm" type="button" onClick={onContinue}>
            Continue
          </Button>
        </WizardFooter>
      </div>
    )
  }

  return (
    <Form {...methods}>
      <form
        autoComplete="off"
        onSubmit={e => {
          e.preventDefault()
          submit()
        }}
        className="space-y-5"
      >
        <p className="text-center text-sm text-muted-foreground">{connect.credentialsHelp(ctx)}</p>
        <CredentialFields
          connect={connect}
          fields={connect.credentialFields}
          control={methods.control as unknown as Control<CredentialValues>}
          ctx={ctx}
          mode="create"
          onSubmit={submit}
        />
        {error && <Alert variant="destructive">{error}</Alert>}
        <WizardFooter>
          <Button
            variant="secondary"
            size="sm"
            type="button"
            onClick={() => (connector.hasCredentials ? setEditing(false) : onBack())}
          >
            {connector.hasCredentials ? 'Cancel' : 'Back'}
          </Button>
          <Button
            size="sm"
            type="submit"
            hasIcon={updateMut.isPending}
            disabled={updateMut.isPending || !methods.formState.isValid}
          >
            {updateMut.isPending && <Spinner />}
            Verify credentials
          </Button>
        </WizardFooter>
      </form>
    </Form>
  )
}

const WebhookStep = ({ definition, ctx, connector, onBack, onSaved, onContinue }: StepProps) => {
  const { connect, name } = definition
  const webhook = connect.webhook!
  const field = connect.fields[webhook.secretField]
  const [secret, setSecret] = useState('')
  const [error, setError] = useState<string>()
  // Stripe endpoints are registered by the backend; a pasted secret is the fallback.
  const [manualOpen, setManualOpen] = useState(webhook.mode === 'manual' || !connector.hasWebhookSecret)
  const updateMut = useMutation(updatePaymentProviderCredentials)

  const saveSecret = async (): Promise<boolean> => {
    const parsed = field.rule.safeParse(secret)
    if (!parsed.success) {
      setError(parsed.error.issues[0]?.message ?? 'Invalid secret')
      return false
    }
    setError(undefined)
    try {
      // Blank credentials keep the stored ones, so only the secret changes.
      const res = await updateMut.mutateAsync({
        id: connector.id,
        credentials: connect.toCredentials({ [webhook.secretField]: secret }, ctx),
      })
      onSaved(res.connector)
      setSecret('')
      return true
    } catch (e) {
      setError(userFacingError(e, "Couldn't save the signing secret.", name))
      return false
    }
  }

  const onNext = async () => {
    if (secret) {
      if (await saveSecret()) onContinue()
    } else if (connector.hasWebhookSecret) {
      onContinue()
    } else {
      setError('Paste the signing secret to continue.')
    }
  }

  const url = connector.webhookUrl ? (
    <CopyToClipboardButton
      text={connector.webhookUrl}
      buttonClassName="max-w-full h-auto items-start text-left whitespace-normal"
      className="whitespace-normal break-all"
    />
  ) : (
    <p className="text-xs text-muted-foreground">The webhook URL isn&apos;t available yet.</p>
  )

  const secretInput = (
    <div className="space-y-1.5">
      <Label htmlFor="webhook-secret" className="text-xs font-normal">
        {field.label}
      </Label>
      <Input
        id="webhook-secret"
        value={secret}
        onChange={e => setSecret(e.target.value.trim())}
        placeholder={
          connector.hasWebhookSecret
            ? 'Leave blank to keep the saved secret'
            : typeof field.placeholder === 'function'
              ? field.placeholder(ctx)
              : field.placeholder
        }
        showPasswordToggle
        autoComplete="off"
        data-form-type="other"
      />
    </div>
  )

  return (
    <div className="space-y-5">
      {webhook.mode === 'auto' && connector.hasWebhookSecret && (
        <div className="space-y-1.5 rounded-lg border border-success/40 bg-success/5 p-4">
          <p className="flex items-center gap-2 text-sm font-medium">
            <CheckCircle2 className="h-4 w-4 text-success" />
            Webhook endpoint configured in your {name} account
          </p>
          <p className="break-all font-mono text-xs text-muted-foreground">
            {connector.webhookUrl}
          </p>
          <p className="text-xs text-muted-foreground">
            Subscribed to payments, mandates, refunds and disputes.
          </p>
        </div>
      )}

      {webhook.mode === 'manual' ? (
        <ol className="list-decimal space-y-4 pl-5 text-sm">
          <li className="space-y-2">
            <p>{webhook.instructions(ctx)}</p>
            {url}
          </li>
          <li className="space-y-2">
            <p>Paste its signing secret:</p>
            {secretInput}
          </li>
        </ol>
      ) : (
        <details
          open={manualOpen}
          onToggle={e => setManualOpen((e.target as HTMLDetailsElement).open)}
          className="rounded-md border border-border bg-card px-3 py-2"
        >
          <summary className="cursor-pointer text-xs text-muted-foreground">
            My API key can&apos;t create webhook endpoints
          </summary>
          <div className="space-y-3 pt-3 text-xs text-muted-foreground">
            <p>{webhook.instructions(ctx)}</p>
            {url}
            {secretInput}
            <div className="flex justify-end">
              <Button
                size="sm"
                variant="secondary"
                type="button"
                disabled={!secret || updateMut.isPending}
                onClick={saveSecret}
              >
                Save secret
              </Button>
            </div>
          </div>
        </details>
      )}

      {error && <Alert variant="destructive">{error}</Alert>}

      <WizardFooter>
        <Button variant="secondary" size="sm" type="button" onClick={onBack}>
          Back
        </Button>
        <Button
          size="sm"
          type="button"
          onClick={onNext}
          hasIcon={updateMut.isPending}
          disabled={updateMut.isPending}
        >
          {updateMut.isPending && <Spinner />}
          Continue
        </Button>
      </WizardFooter>
    </div>
  )
}

const FinishLaterPrompt = ({
  visible,
  connector,
  onKeepEditing,
  onClose,
}: {
  visible: boolean
  connector: Connector
  onKeepEditing: () => void
  onClose: () => void
}) => {
  const invalidate = useInvalidateConnectors()
  const deleteMut = useMutation(deleteConnector, { onSuccess: invalidate })

  const discard = async () => {
    try {
      await deleteMut.mutateAsync({ id: connector.id })
      toast.success('Draft discarded')
      onClose()
    } catch (e) {
      toast.error(userFacingError(e, "Couldn't discard the draft."))
    }
  }

  return (
    <Modal
      visible={visible}
      size="small"
      header="Finish later?"
      onCancel={onKeepEditing}
      customFooter={
        <div className="flex w-full items-center justify-between gap-2">
          <Button
            variant="destructiveGhost"
            size="sm"
            onClick={discard}
            hasIcon={deleteMut.isPending}
            disabled={deleteMut.isPending}
          >
            {deleteMut.isPending && <Spinner />}
            Discard draft
          </Button>
          <div className="flex gap-2">
            <Button variant="secondary" size="sm" onClick={onKeepEditing}>
              Keep editing
            </Button>
            <Button size="sm" onClick={onClose}>
              Close
            </Button>
          </div>
        </div>
      }
    >
      <Modal.Content>
        <p className="py-4 text-sm text-muted-foreground">
          Your progress on <span className="font-medium text-foreground">{connector.alias}</span> is
          saved. It won&apos;t collect payments until you activate it. You can resume from
          Integrations.
        </p>
      </Modal.Content>
    </Modal>
  )
}
