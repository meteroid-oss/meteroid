import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  GenericFormField,
  Input,
  Label,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { RefreshCwIcon, UploadIcon, XIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Control, useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { InlineLoading } from '@/components/Loading'
import { Slot, useModule } from '@/edition'
import { TEMPLATE_LOCALES } from '@/features/customer-emails/locales'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import { LocaleToggles, sortLocales } from '@/features/settings/components/LocaleToggles'
import { PortalAccentField } from '@/features/settings/components/PortalAccentField'
import { PortalBrandingPreview } from '@/features/settings/components/PortalBrandingPreview'
import {
  SettingsFieldGrid,
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { UnsavedChangesBar } from '@/features/settings/components/UnsavedChangesBar'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { localeLabel } from '@/features/settings/utils/localeRecord'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useZodForm } from '@/hooks/useZodForm'
import { env } from '@/lib/env'
import {
  getEmailSettings,
  previewEmail,
  previewInvoiceEmail,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import {
  getInvoicingEntity,
  listInvoicingEntities,
  updateInvoicingEntity,
  uploadInvoicingEntityLogo,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { InvoicingEntity } from '@/rpc/api/invoicingentities/v1/models_pb'

const HEX_COLOR = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i

const hexColor = (example: string) =>
  z
    .union([z.literal(''), z.string().regex(HEX_COLOR, `Enter a valid hex color like ${example}`)])
    .optional()

const brandingSchema = z.object({
  brandColor: hexColor('#C6F94E'),
  brandColorLight: hexColor('#2E7D32'),
  portalThemeMode: z.string().optional(),
  portalRoundness: z.string().optional(),
  customerLocale: z.string(),
  customerExtraLocales: z.array(z.string()),
})

// WCAG relative luminance, as the mailer computes it for its 3:1 rule on white.
const luminance = (hex: string) => {
  const h = hex.slice(1)
  const full = h.length === 3 ? [...h].map(c => c + c).join('') : h
  const [r, g, b] = [0, 2, 4].map(i => {
    const c = parseInt(full.slice(i, i + 2), 16) / 255
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

const isLegibleOnWhite = (hex: string) => 1.05 / (luminance(hex) + 0.05) >= 3

// Readable label on a button of `hex`, mirroring the mailer's on_color.
const textOn = (hex: string) => (luminance(hex) > 0.179 ? '#111318' : '#ffffff')

// Radix Select rejects '' as an item value; saved as '' (customer's language, else the invoice language).
const SAME_AS_INVOICE = 'invoice'

// The customer default the form currently resolves to, for the extras' exclusion.
const customerDefault = (customerLocale?: string, entity?: InvoicingEntity) =>
  customerLocale && customerLocale !== SAME_AS_INVOICE
    ? customerLocale
    : entity?.effectiveDocumentLanguage || undefined

export const BrandingTab = () => {
  const queryClient = useQueryClient()
  const {
    selectedEntityId: invoiceEntityId,
    isLoading,
    currentEntity: currentInvoicingEntity,
  } = useInvoicingEntity()

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntity, {
    onSuccess: async res => {
      if (res.entity) {
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: listInvoicingEntities,
            cardinality: undefined,
          }),
        })
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: getInvoicingEntity,
            input: { id: res.entity?.id },
            cardinality: 'finite',
          }),
        })
        // Emails render in the customer-facing languages: refetch settings and previews.
        await Promise.all([
          queryClient.invalidateQueries({
            queryKey: createConnectQueryKey({ schema: getEmailSettings, cardinality: undefined }),
          }),
          queryClient.invalidateQueries({
            queryKey: createConnectQueryKey({ schema: previewEmail, cardinality: undefined }),
          }),
          queryClient.invalidateQueries({
            queryKey: createConnectQueryKey({
              schema: previewInvoiceEmail,
              cardinality: undefined,
            }),
          }),
        ])
        toast.success('Branding updated')
      }
    },
  })

  const methods = useZodForm({ schema: brandingSchema })
  const [brandColor, brandColorLight, portalThemeMode, portalRoundness, customerLocale] = useWatch({
    control: methods.control,
    name: ['brandColor', 'brandColorLight', 'portalThemeMode', 'portalRoundness', 'customerLocale'],
  })

  useEffect(() => {
    if (currentInvoicingEntity) {
      methods.reset({
        brandColor: currentInvoicingEntity.brandColor || '',
        brandColorLight: currentInvoicingEntity.brandColorLight || '',
        portalThemeMode: currentInvoicingEntity.portalThemeMode || 'dark',
        portalRoundness: currentInvoicingEntity.portalRoundness || 'Modern',
        customerLocale: currentInvoicingEntity.customerLocale || SAME_AS_INVOICE,
        customerExtraLocales: sortLocales(
          currentInvoicingEntity.customerExtraLocales ?? [],
          currentInvoicingEntity.effectiveCustomerLocale
        ),
      })
    } else {
      methods.reset()
    }
  }, [currentInvoicingEntity])

  const themingShipped = useModule('portal') !== undefined
  const theming = useEntitlement(entitlementCodes.PORTAL_THEMING)

  if (isLoading) {
    return <InlineLoading />
  }

  const brandHex = brandColor && HEX_COLOR.test(brandColor) ? brandColor : undefined
  const brandLegible = !!brandHex && isLegibleOnWhite(brandHex)
  const lightHex = brandColorLight && HEX_COLOR.test(brandColorLight) ? brandColorLight : undefined
  // What the email button actually uses (mailer rules): light color, legible brand, else black.
  const emailButtonColor = lightHex ?? (brandHex && brandLegible ? brandHex : '#000000')
  // Only relevant when the brand color can't carry a button on white, or one is already saved.
  const showLightColor = (!!brandHex && !brandLegible) || !!brandColorLight

  const defaultCustomerLocale = customerDefault(customerLocale, currentInvoicingEntity)
  const invoiceLocale = currentInvoicingEntity?.effectiveDocumentLanguage

  const onSubmit = async (values: z.infer<typeof brandingSchema>) => {
    await updateInvoicingEntityMut.mutateAsync({
      id: invoiceEntityId,
      data: {
        brandColor: values.brandColor,
        brandColorLight: values.brandColorLight,
        portalThemeMode: theming ? values.portalThemeMode : undefined,
        portalRoundness: theming ? values.portalRoundness : undefined,
        customerLocale: values.customerLocale === SAME_AS_INVOICE ? '' : values.customerLocale,
        customerExtraLocales: {
          values: sortLocales(
            values.customerExtraLocales,
            customerDefault(values.customerLocale, currentInvoicingEntity)
          ),
        },
      },
    })
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)} className="flex max-w-3xl flex-col gap-6">
        <SettingsSection
          divided={false}
          title="Portal & invoice branding"
          description="Defaults for the hosted customer portal, checkout and invoice payment pages. Applied per invoicing entity; non-default entities inherit blank values from your default."
          action={<InvoicingEntitySelect />}
        >
          <div className="space-y-4">
            <SettingsRowGroup>
              <SettingsRow
                title="Logo"
                description="Shown on invoices, emails and portals. PNG or JPEG, max 1 MB."
              >
                {currentInvoicingEntity && <FileUpload entity={currentInvoicingEntity} />}
              </SettingsRow>
            </SettingsRowGroup>

            <SettingsFieldGrid>
              {themingShipped && (
                <>
                  <SelectFormField
                    name="portalThemeMode"
                    control={methods.control}
                    label="Theme"
                    placeholder="Dark"
                    disabled={!theming}
                  >
                    <SelectItem value="light">Light</SelectItem>
                    <SelectItem value="dark">Dark</SelectItem>
                  </SelectFormField>
                  <SelectFormField
                    name="portalRoundness"
                    control={methods.control}
                    label="Roundness"
                    placeholder="Modern"
                    disabled={!theming}
                  >
                    <SelectItem value="Sharp">Sharp</SelectItem>
                    <SelectItem value="Modern">Modern</SelectItem>
                    <SelectItem value="Rounded">Rounded</SelectItem>
                  </SelectFormField>
                </>
              )}
              <PortalAccentField
                name="brandColor"
                control={methods.control}
                themeMode={portalThemeMode}
                containerClassName="sm:col-span-2"
                hint={
                  showLightColor && (
                    <EmailButtonColor
                      control={methods.control}
                      custom={!!brandColorLight}
                      buttonColor={emailButtonColor}
                      onReset={() => methods.setValue('brandColorLight', '', { shouldDirty: true })}
                    />
                  )
                }
              />
              <div className="sm:col-span-2">
                <PortalBrandingPreview
                  accent={brandColor}
                  theme={portalThemeMode}
                  roundness={portalRoundness}
                />
              </div>
            </SettingsFieldGrid>
          </div>
        </SettingsSection>

        <SettingsSection
          title="Customer-facing language"
          description="The language of emails and the customer portal. The invoice PDF keeps the invoice language."
        >
          <SettingsFieldGrid>
            <SelectFormField
              name="customerLocale"
              control={methods.control}
              label="Default language"
            >
              <SelectItem value={SAME_AS_INVOICE}>
                {invoiceLocale
                  ? `Each customer's preferred language, else ${localeLabel(invoiceLocale)}`
                  : "Each customer's preferred language"}
              </SelectItem>
              {TEMPLATE_LOCALES.map(locale => (
                <SelectItem key={locale} value={locale}>
                  {localeLabel(locale)}
                </SelectItem>
              ))}
            </SelectFormField>
            <GenericFormField
              name="customerExtraLocales"
              control={methods.control}
              label="Additional languages"
              description={
                customerLocale === SAME_AS_INVOICE
                  ? `Languages you write reminder copy in. Customers otherwise get emails in their preferred language; a reminder without copy in it goes out in ${
                      defaultCustomerLocale
                        ? localeLabel(defaultCustomerLocale)
                        : 'the default language'
                    }.`
                  : `Customers whose preferred language is enabled get it; everyone else gets ${
                      defaultCustomerLocale
                        ? localeLabel(defaultCustomerLocale)
                        : 'the default language'
                    }. Custom reminder copy must be written in every enabled language.`
              }
              containerClassName="sm:col-span-2"
              render={({ field }) => (
                <LocaleToggles
                  options={TEMPLATE_LOCALES.filter(l => l !== defaultCustomerLocale)}
                  value={(field.value ?? []).filter(l => l !== defaultCustomerLocale)}
                  onChange={field.onChange}
                  disabled={!currentInvoicingEntity}
                />
              )}
            />
          </SettingsFieldGrid>
        </SettingsSection>

        <Slot name="brandingSettings.sections" />

        <UnsavedChangesBar
          show={methods.formState.isDirty}
          saving={updateInvoicingEntityMut.isPending}
          onDiscard={() => methods.reset()}
        />
      </form>
    </Form>
  )
}

// Compact override under the brand color: only shown when that color can't carry a button on
// white. Previews the email button and edits `brandColorLight` with a swatch + hex input.
const EmailButtonColor = ({
  control,
  custom,
  buttonColor,
  onReset,
}: {
  control: Control<z.infer<typeof brandingSchema>>
  custom: boolean
  buttonColor: string
  onReset: () => void
}) => (
  <div className="flex flex-col gap-3 rounded-md border border-border bg-muted/30 p-3 sm:flex-row sm:items-center">
    <div className="flex shrink-0 items-center justify-center rounded border border-border bg-white px-4 py-3">
      <span
        className="rounded px-3 py-1 text-xs font-semibold"
        style={{ backgroundColor: buttonColor, color: textOn(buttonColor) }}
      >
        Pay invoice
      </span>
    </div>
    <div className="flex min-w-0 flex-1 flex-col gap-2">
      <p className="text-xs text-muted-foreground">
        {custom
          ? 'Emails use this color for buttons on white backgrounds.'
          : 'Too light for white backgrounds: emails keep it as an accent and use a black button. Pick an email button color to override.'}
      </p>
      <GenericFormField
        name="brandColorLight"
        control={control}
        render={({ field }) => (
          <div className="flex items-center gap-2">
            <label
              className="relative h-7 w-7 shrink-0 cursor-pointer overflow-hidden rounded-md border border-border"
              title="Email button color"
              style={{ backgroundColor: buttonColor }}
            >
              <input
                type="color"
                value={HEX_COLOR.test(field.value ?? '') ? (field.value as string) : buttonColor}
                onChange={e => field.onChange(e.target.value.toUpperCase())}
                className="absolute -inset-2 h-[200%] w-[200%] cursor-pointer opacity-0"
              />
            </label>
            <Input
              value={field.value ?? ''}
              placeholder="#000000"
              onChange={e => field.onChange(e.target.value)}
              onBlur={field.onBlur}
              className="h-8 w-28 font-mono uppercase"
              aria-label="Email button color"
            />
            {custom && (
              <Button type="button" variant="ghost" size="sm" className="h-8" onClick={onReset}>
                Use black
              </Button>
            )}
          </div>
        )}
      />
    </div>
  </div>
)

const FileUpload = ({ entity }: { entity: InvoicingEntity }) => {
  const [isUploading, setIsUploading] = useState(false)
  const queryClient = useQueryClient()

  const updateInvoicingEntityMut = useMutation(uploadInvoicingEntityLogo, {
    onSuccess: async () => {
      // useInvoicingEntity derives the shown logo from listInvoicingEntities, so
      // that's the query to refetch — invalidating getInvoicingEntity alone left
      // the preview stale after upload/clear.
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listInvoicingEntities,
          cardinality: undefined,
        }),
      })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoicingEntity,
          cardinality: undefined,
        }),
      })
      toast.success('Logo updated')
    },
  })

  const handleFileUpload = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    if (!file) return

    setIsUploading(true)

    const reader = new FileReader()
    reader.onload = async () => {
      const arrayBuffer = reader.result as ArrayBuffer
      const uint8Array = new Uint8Array(arrayBuffer)

      try {
        await updateInvoicingEntityMut.mutateAsync({
          id: entity.id,
          file: { data: uint8Array },
        })
      } catch (error) {
        console.error('Error uploading file:', error)
        toast.error('Failed to upload file. Please try again.')
      } finally {
        setIsUploading(false)
      }
    }

    reader.readAsArrayBuffer(file)
  }

  const clearFile = async () => {
    setIsUploading(true)
    try {
      await updateInvoicingEntityMut.mutateAsync({ id: entity.id, file: undefined })
    } catch (error) {
      toast.error('Failed to update logo.')
    } finally {
      setIsUploading(false)
    }
  }

  return (
    <div className="flex flex-row items-center gap-2">
      {entity.logoAttachmentId && (
        <div className="group relative h-10 overflow-hidden rounded border border-border">
          <img
            src={env.meteroidRestApiUri + '/files/v1/logo/' + entity.logoAttachmentId}
            alt="Entity logo"
            className="h-full w-full object-cover"
          />
          <Button
            size="icon"
            variant="destructive"
            type="button"
            className="absolute right-0 top-0 h-5 w-5 p-1 opacity-0 transition-opacity group-hover:opacity-100"
            onClick={clearFile}
          >
            <XIcon size="14" />
          </Button>
        </div>
      )}
      <Input
        type="file"
        id="logo-upload"
        className="hidden"
        accept="image/*"
        onChange={handleFileUpload}
      />
      <Button size="sm" variant="outline" type="button" asChild>
        <Label htmlFor="logo-upload" className="cursor-pointer">
          {isUploading ? (
            <RefreshCwIcon size={14} className="mr-1.5 animate-spin" />
          ) : (
            <UploadIcon size={14} className="mr-1.5" />
          )}
          Upload
        </Label>
      </Button>
    </div>
  )
}
