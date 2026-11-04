import { Code } from '@connectrpc/connect'
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { Button, Form, GenericFormField, InputFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { ValuesTagInput } from '@/components/form/ValuesTagInput'
import { Slot } from '@/edition'
import {
  SettingsFieldGrid,
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { localeLabel } from '@/features/settings/utils/localeRecord'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsCloud } from '@/hooks/useIsCloud'
import { usePermissions } from '@/hooks/usePermissions'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import {
  getEmailSettings,
  previewEmail,
  previewInvoiceEmail,
  updateEmailSettings,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { EmailSettings } from '@/rpc/api/customeremails/v1/models_pb'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

import { emailSettingsQueryKey } from './EmailKindsView'

const MAX_BCC = 5

const senderSchema = z.object({
  senderName: z.string().max(80, 'Keep the sender name under 80 characters'),
  replyTo: z.union([z.literal(''), z.string().email('Enter a valid email address')]),
  bcc: z
    .array(z.string().email('Every Bcc entry must be an email address'))
    .max(MAX_BCC, `Up to ${MAX_BCC} Bcc addresses`),
})

type SenderForm = z.infer<typeof senderSchema>

const toForm = (settings?: EmailSettings): SenderForm => ({
  senderName: settings?.senderName ?? '',
  replyTo: settings?.replyTo ?? '',
  bcc: settings?.bcc ?? [],
})

const senderAddressDescription = (isCloud?: boolean, mailerEnabled?: boolean) => {
  if (isCloud === undefined) return undefined
  if (isCloud) {
    return 'Uses the shared Meteroid domain. The sender name below is shown in front of it.'
  }
  if (!mailerEnabled) {
    return 'Email delivery is not configured on this instance (MAILER_SMTP_HOST).'
  }
  return 'Set by the instance (MAILER_CUSTOMER_FROM_ADDR). The sender name below is shown in front of it.'
}

export const SenderView = () => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const basePath = useBasePath()
  const { selectedEntityId: entityId, currentEntity } = useInvoicingEntity()
  const canManage = usePermissions().has('settings:manage')
  const isCloud = useIsCloud()
  const mailerEnabled = useQuery(getInstance).data?.mailerEnabled

  const settingsQuery = useQuery(
    getEmailSettings,
    entityId ? { invoicingEntityId: entityId } : skipToken,
    { retry: (count, err) => err.code !== Code.Unimplemented && count < 2 }
  )
  const settings = settingsQuery.data?.settings
  const unavailable = settingsQuery.error?.code === Code.Unimplemented

  const updateMut = useMutation(updateEmailSettings, {
    onSuccess: async () => {
      if (entityId) {
        await queryClient.invalidateQueries({ queryKey: emailSettingsQueryKey(entityId) })
      }
      // Previews render the sender name and reply-to: refetch every cached one.
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: previewEmail, cardinality: undefined }),
        }),
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: previewInvoiceEmail, cardinality: undefined }),
        }),
      ])
      toast.success('Email settings updated')
    },
    onError: err => toast.error(err.rawMessage || 'Could not save the email settings'),
  })

  const methods = useZodForm({ schema: senderSchema, defaultValues: toForm() })
  const { reset } = methods

  useEffect(() => {
    reset(toForm(settings))
  }, [settings, reset])

  // Settings tabs are path segments (`settings/:tab`).
  const goToTab = (tab: string) => navigate(`${basePath}/settings/${tab}`, { replace: true })

  const onSubmit = async (values: SenderForm) => {
    if (!entityId) return
    await updateMut.mutateAsync({
      invoicingEntityId: entityId,
      data: {
        senderName: values.senderName.trim(),
        replyTo: values.replyTo.trim(),
        bcc: { values: values.bcc },
      },
    })
  }

  const readOnly = !canManage || unavailable
  const defaultLocale = settings?.effectiveLocale
  const extraLocales = settings?.extraLocales ?? []
  const logoUrl = currentEntity?.logoAttachmentId
    ? `${env.meteroidRestApiUri}/files/v1/logo/${currentEntity.logoAttachmentId}`
    : undefined
  const address = [currentEntity?.city, currentEntity?.country].filter(Boolean).join(', ')

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)} className="flex max-w-3xl flex-col gap-6">
        <SettingsSection
          divided={false}
          title="Sender"
          description="How customer emails from this invoicing entity are addressed."
          action={
            canManage && (
              <Button
                type="submit"
                size="sm"
                disabled={readOnly || !methods.formState.isDirty || updateMut.isPending}
              >
                Save
              </Button>
            )
          }
        >
          {unavailable && (
            <p className="mb-3 text-[13px] text-muted-foreground">
              Email settings are not available on this server yet.
            </p>
          )}
          <div className="space-y-4">
            <SettingsRowGroup>
              <SettingsRow
                title="Sender address"
                description={senderAddressDescription(isCloud, mailerEnabled)}
              >
                {mailerEnabled === false ? (
                  <span className="text-xs text-muted-foreground">Not configured</span>
                ) : (
                  <code className="rounded bg-muted px-1.5 py-0.5 text-xs">
                    {settings?.fromAddress || '—'}
                  </code>
                )}
              </SettingsRow>
            </SettingsRowGroup>

            <SettingsFieldGrid>
              <InputFormField
                name="senderName"
                control={methods.control}
                label="Sender name"
                placeholder={currentEntity?.legalName ?? 'Acme Inc.'}
                disabled={readOnly}
              />
              <InputFormField
                name="replyTo"
                control={methods.control}
                label="Reply-to"
                placeholder="billing@example.com"
                disabled={readOnly}
              />
              <GenericFormField
                name="bcc"
                control={methods.control}
                label="Bcc"
                description={`Every customer email is copied to these addresses (up to ${MAX_BCC}).`}
                containerClassName="sm:col-span-2"
                render={({ field }) => (
                  <ValuesTagInput
                    value={field.value}
                    onChange={v => field.onChange(v.slice(0, MAX_BCC))}
                    placeholder="Add an address and press Enter"
                  />
                )}
              />
            </SettingsFieldGrid>
          </div>
        </SettingsSection>

        <SettingsSection
          title="Languages"
          description="Each email goes out in a single language, chosen per customer from the entity's customer-facing languages."
        >
          <SettingsRowGroup>
            <SettingsRow
              title="Default language"
              description={
                <button
                  type="button"
                  className="underline-offset-2 hover:underline"
                  onClick={() => goToTab('branding')}
                >
                  Change in Branding
                </button>
              }
            >
              <span className="text-xs">{defaultLocale ? localeLabel(defaultLocale) : '—'}</span>
            </SettingsRow>
            <SettingsRow
              title="Additional languages"
              description={
                <button
                  type="button"
                  className="underline-offset-2 hover:underline"
                  onClick={() => goToTab('branding')}
                >
                  Change in Branding
                </button>
              }
            >
              {extraLocales.length > 0 ? (
                <span className="max-w-[260px] text-right text-xs">
                  {extraLocales.map(localeLabel).join(', ')}
                </span>
              ) : (
                <span className="text-xs text-muted-foreground">{settings ? 'None' : '—'}</span>
              )}
            </SettingsRow>
          </SettingsRowGroup>
        </SettingsSection>

        <SettingsSection
          title="Branding in emails"
          description="Emails reuse the entity's invoice branding; change it from the linked tabs."
        >
          <SettingsRowGroup>
            <SettingsRow
              title="Logo"
              description={
                <button
                  type="button"
                  className="underline-offset-2 hover:underline"
                  onClick={() => goToTab('branding')}
                >
                  Change in Branding
                </button>
              }
            >
              {logoUrl ? (
                <img src={logoUrl} alt="Logo" className="h-8 max-w-[120px] object-contain" />
              ) : (
                <span className="text-xs text-muted-foreground">No logo</span>
              )}
            </SettingsRow>
            <SettingsRow
              title="Brand colour"
              description={
                <button
                  type="button"
                  className="underline-offset-2 hover:underline"
                  onClick={() => goToTab('branding')}
                >
                  Change in Branding
                </button>
              }
            >
              {currentEntity?.brandColor ? (
                <span className="flex items-center gap-2 text-xs">
                  <span
                    className="h-4 w-4 rounded border border-border"
                    style={{ backgroundColor: currentEntity.brandColor }}
                  />
                  {currentEntity.brandColor}
                </span>
              ) : (
                <span className="text-xs text-muted-foreground">Default</span>
              )}
            </SettingsRow>
            <SettingsRow
              title="Footer"
              description={
                <button
                  type="button"
                  className="underline-offset-2 hover:underline"
                  onClick={() => goToTab('merchant')}
                >
                  Legal name and address, from the Merchant tab
                </button>
              }
            >
              <span className="max-w-[260px] truncate text-xs text-muted-foreground">
                {[currentEntity?.legalName, address].filter(Boolean).join(' · ') || '—'}
              </span>
            </SettingsRow>
            <Slot name="emailSenderSettings.brandingRows" />
          </SettingsRowGroup>
        </SettingsSection>
      </form>
    </Form>
  )
}
