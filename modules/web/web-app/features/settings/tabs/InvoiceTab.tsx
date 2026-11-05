import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  cn,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Form,
  InputFormField,
  Switch,
  TextareaFormField,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, PlusIcon, XIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { FieldPath, useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import {
  EInvoicingFormatSelect,
  EINVOICING_FORMAT_OFF,
  einvoicingFormatForPatch,
  einvoicingFormatFromStored,
} from '@/components/EInvoicingFormatSelect'
import {
  INVOICE_LANGUAGE_DEFAULT,
  INVOICE_LANGUAGE_OPTIONS,
  InvoiceLanguageSelect,
  invoiceLanguageForPatch,
  invoiceLanguageFromStored,
  invoiceLanguageIsUnsafeGuess,
} from '@/components/InvoiceLanguageSelect'
import { Loading } from '@/components/Loading'
import { Slot } from '@/edition'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import {
  SettingsFieldGrid,
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import {
  hasFooterOverride,
  seedFooterForm,
  toByLocale,
  visibleFooterLocales,
} from '@/features/settings/utils/footerTranslations'
import { useZodForm } from '@/hooks/useZodForm'
import {
  listInvoicingEntities,
  updateInvoicingEntity,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

const invoiceDetailsSchema = z.object({
  invoiceNumberPattern: z.string().optional(),
  creditNoteNumberPattern: z
    .string()
    .regex(/\{number(:\d+)?\}/, 'Must contain a {number} or {number:N} placeholder')
    .optional(),
  gracePeriodHours: z.number().optional(),
  netTerms: z.number().optional(),
  footerTranslations: z
    .record(z.string(), z.object({ info: z.string().optional(), legal: z.string().optional() }))
    .optional(),
  logoAttachmentId: z.string().optional(),
  requireBillingInformation: z.boolean().optional(),
  invoicingLanguage: z.string().optional(),
  einvoicingFormat: z.string().optional(),
})

type InvoiceForm = z.infer<typeof invoiceDetailsSchema>

// Footer language switcher: every language edits an entry in `footerTranslations`,
// and the entity's home language additionally mirrors into the untagged base
// footer. Real locales only — derived from the supported-language source, dropping
// the abstract "default" sentinel so they never drift.
const FOOTER_LOCALES = INVOICE_LANGUAGE_OPTIONS.map(o => o.value).filter(
  v => v !== INVOICE_LANGUAGE_DEFAULT
)
const footerLocaleLabel = (value: string): string =>
  INVOICE_LANGUAGE_OPTIONS.find(o => o.value === value)?.label ?? value

export const InvoiceTab = () => {
  const queryClient = useQueryClient()
  const { selectedEntityId: invoiceEntityId, isLoading, currentEntity } = useInvoicingEntity()

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntity, {
    onSuccess: async res => {
      if (res.entity) {
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: listInvoicingEntities,
            cardinality: undefined,
          }),
        })
        toast.success('Invoicing entity updated')
      }
    },
  })

  const methods = useZodForm({
    schema: invoiceDetailsSchema,
  })

  const requireBillingInformation = useWatch({
    control: methods.control,
    name: 'requireBillingInformation',
  })

  const einvoicingFormat = useWatch({ control: methods.control, name: 'einvoicingFormat' })
  // Every language edits `footerTranslations[locale]`, so no text sits in a slot
  // whose meaning moves. `homeLang` owns the untagged base footer — the fallback a
  // language prints wherever its own field is empty (`FooterTranslations::resolve`).
  const selectedLang = useWatch({ control: methods.control, name: 'invoicingLanguage' })
  const homeLang = currentEntity?.countryDocumentLanguage || FOOTER_LOCALES[0]
  // The language invoices are issued in, following the unsaved select: an explicit
  // choice, or "inherit", which resolves to the country language — `homeLang` by
  // definition. Before the form has loaded, the saved answer.
  const effectiveLang =
    selectedLang === undefined
      ? currentEntity?.effectiveDocumentLanguage || homeLang
      : FOOTER_LOCALES.includes(selectedLang)
        ? selectedLang
        : homeLang

  // View-only state — switching or adding a tab never dirties the form.
  const [activeLang, setActiveLang] = useState<string>(effectiveLang)
  const [addedLangs, setAddedLangs] = useState<string[]>([])
  const footerTranslations = useWatch({ control: methods.control, name: 'footerTranslations' })

  const footerTabs = visibleFooterLocales(
    FOOTER_LOCALES,
    homeLang,
    effectiveLang,
    footerTranslations,
    addedLangs
  )
  const addableLangs = FOOTER_LOCALES.filter(l => !footerTabs.includes(l))
  // The active tab can disappear from under the operator: changing the invoice
  // language away from an empty language they were looking at. Follow the invoice
  // language rather than keep editing a tab that is no longer shown.
  const shownLang = footerTabs.includes(activeLang) ? activeLang : effectiveLang
  // Make the fallback stick. Otherwise the stale choice resurfaces and steals the
  // view the moment its tab comes back — switching the invoice language back to it
  // while the operator is editing another language.
  useEffect(() => {
    if (shownLang !== activeLang) setActiveLang(shownLang)
  }, [shownLang, activeLang])

  const addFooterLang = (locale: string) => {
    setAddedLangs(langs => [...langs, locale])
    setActiveLang(locale)
  }
  // Clearing both fields is the whole removal: blank locales are pruned on save, and
  // the language falls back to the default footer again. If its tab was the active
  // one, `shownLang` moves to the invoice language on its own.
  const removeFooterLang = (locale: string) => {
    methods.setValue(
      'footerTranslations',
      { ...methods.getValues('footerTranslations'), [locale]: { info: '', legal: '' } },
      { shouldDirty: true }
    )
    setAddedLangs(langs => langs.filter(l => l !== locale))
  }

  const homeFooter = footerTranslations?.[homeLang]
  const shownFooter = footerTranslations?.[shownLang]
  const onHomeTab = shownLang === homeLang
  // Placeholder on an empty translated field: the default text that will print in
  // its place, so the fallback is visible field by field, as it is applied.
  const fallbackPlaceholder = (field: 'info' | 'legal') =>
    onHomeTab ? undefined : homeFooter?.[field] || undefined

  const footerInfoName = `footerTranslations.${shownLang}.info` as FieldPath<InvoiceForm>
  const footerLegalName = `footerTranslations.${shownLang}.legal` as FieldPath<InvoiceForm>

  useEffect(() => {
    if (currentEntity) {
      const home = currentEntity.countryDocumentLanguage || FOOTER_LOCALES[0]
      methods.reset({
        invoiceNumberPattern: currentEntity.invoiceNumberPattern || '',
        creditNoteNumberPattern: currentEntity.creditNoteNumberPattern || '',
        gracePeriodHours: currentEntity.gracePeriodHours,
        netTerms: currentEntity.netTerms,
        footerTranslations: seedFooterForm(
          FOOTER_LOCALES,
          currentEntity.invoiceFooterTranslations?.byLocale,
          home,
          currentEntity.invoiceFooterInfo,
          currentEntity.invoiceFooterLegal
        ),
        logoAttachmentId: currentEntity.logoAttachmentId || '',
        requireBillingInformation: currentEntity.requireBillingInformation ?? false,
        invoicingLanguage: invoiceLanguageFromStored(currentEntity.invoicingLanguage),
        einvoicingFormat: einvoicingFormatFromStored(currentEntity.einvoicingFormat),
      })
      setActiveLang(currentEntity.effectiveDocumentLanguage || home)
      setAddedLangs([])
    } else {
      methods.reset()
    }
  }, [currentEntity])

  if (isLoading) {
    return <Loading />
  }

  const onSubmit = async (values: z.infer<typeof invoiceDetailsSchema>) => {
    // TODO filter out if it hasn't changed
    await updateInvoicingEntityMut.mutateAsync({
      id: invoiceEntityId,
      data: {
        gracePeriodHours: values.gracePeriodHours,
        // The untagged base footer is what renders for a language we ship no bundle
        // for, so keep it mirroring the home-language text.
        invoiceFooterInfo: values.footerTranslations?.[homeLang]?.info ?? '',
        invoiceFooterLegal: values.footerTranslations?.[homeLang]?.legal ?? '',
        invoiceFooterTranslations: {
          byLocale: toByLocale(values.footerTranslations),
        },
        invoiceNumberPattern: values.invoiceNumberPattern,
        creditNoteNumberPattern: values.creditNoteNumberPattern,
        logoAttachmentId: values.logoAttachmentId,
        netTerms: values.netTerms,
        requireBillingInformation: values.requireBillingInformation,
        invoicingLanguage: invoiceLanguageForPatch(values.invoicingLanguage),
        einvoicingFormat: einvoicingFormatForPatch(values.einvoicingFormat),
      },
    })
  }

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)}>
          <SettingsSection
            divided={false}
            title="Invoice settings"
            description="How invoices are numbered and when they're due."
            action={
              <div className="flex items-center gap-2">
                <InvoicingEntitySelect />
                <Button
                  size="sm"
                  disabled={
                    !methods.formState.isValid ||
                    !methods.formState.isDirty ||
                    updateInvoicingEntityMut.isPending
                  }
                >
                  Save changes
                </Button>
              </div>
            }
          >
            <div className="space-y-4">
              <SettingsFieldGrid>
                <InputFormField
                  name="invoiceNumberPattern"
                  label="Invoice number pattern"
                  control={methods.control}
                  placeholder="ACME Inc."
                  containerClassName="sm:col-span-2"
                  description="Use the following placeholders: {number} - mandatory sequential number ({number:6} pads to 6 digits), {YYYY} - year, {MM} - month, {DD} - day"
                />
                <InputFormField
                  name="creditNoteNumberPattern"
                  label="Credit note number pattern"
                  control={methods.control}
                  placeholder="CN-{number:6}"
                  containerClassName="sm:col-span-2"
                  description="Same placeholders as the invoice number pattern, with its own sequence"
                />
                <InputFormField
                  name="gracePeriodHours"
                  control={methods.control}
                  label="Grace period (hours)"
                  placeholder="24"
                  type="number"
                />
                <InputFormField
                  name="netTerms"
                  control={methods.control}
                  label="Net terms (days)"
                  type="number"
                  placeholder="30"
                />
                <div className="sm:col-span-2 space-y-1.5">
                  <InvoiceLanguageSelect
                    name="invoicingLanguage"
                    control={methods.control}
                    label="Default invoice language"
                  />
                  {/* Belgian language law fixes the invoice language by the issuer's
                      seat of establishment, and a wrong-language invoice is void — so
                      "inherit" quietly resolves a legal question with the country
                      default (Dutch), right for Flanders and void in Wallonia. Warn
                      rather than block: the default IS correct for a Flemish entity,
                      and a required field would hold every unrelated save hostage. */}
                  {invoiceLanguageIsUnsafeGuess(selectedLang, currentEntity?.country) && (
                    <div className="flex items-start gap-1.5 text-[13px] text-amber-700 dark:text-amber-500">
                      <AlertTriangle size={13} className="mt-0.5 shrink-0" />
                      <span>
                        Without an explicit choice, this entity&apos;s invoices default to Dutch.
                        Belgian law fixes the invoice language by your seat of establishment — Dutch
                        in Flanders, French in Wallonia, German in the German-speaking Community,
                        either in Brussels — and an invoice in the wrong language can be void. Pick
                        the language of your region.
                      </span>
                    </div>
                  )}
                </div>
              </SettingsFieldGrid>

              <SettingsRowGroup>
                {invoiceEntityId && (
                  <Slot name="invoiceSettings.rows" invoicingEntityId={invoiceEntityId} />
                )}
                <SettingsRow
                  title="Require billing information at checkout"
                  description="Customers must provide a billing email and a complete billing address (street, postal code, city and country) before they can complete a checkout"
                >
                  <Switch
                    checked={requireBillingInformation ?? false}
                    onCheckedChange={v =>
                      methods.setValue('requireBillingInformation', v, { shouldDirty: true })
                    }
                  />
                </SettingsRow>
              </SettingsRowGroup>
            </div>
          </SettingsSection>

          <SettingsSection
            title="Structured e-invoicing"
            description="Issue every invoice as a machine-readable document alongside the PDF, under the EN 16931 standard: Factur-X / ZUGFeRD, or XRechnung for Germany."
            className="mt-6"
          >
            <div className="space-y-4">
              <SettingsFieldGrid>
                <EInvoicingFormatSelect
                  name="einvoicingFormat"
                  control={methods.control}
                  containerClassName="sm:col-span-2"
                />
              </SettingsFieldGrid>
              {einvoicingFormat && einvoicingFormat !== EINVOICING_FORMAT_OFF && (
                <p className="text-[13px] text-muted-foreground">
                  Invoices are still issued when they cannot be expressed in this format — the
                  missing fields are reported on the invoice instead. Fill them{' '}
                  <b>before finalizing</b>: a finalized invoice is immutable, so correcting a
                  company detail afterwards does not make it compliant.
                </p>
              )}
              {einvoicingFormat === 'XRECHNUNG' && (
                <p className="text-[13px] text-muted-foreground">
                  XRechnung also needs the invoice contact from the Company tab, and a buyer
                  reference on each customer — the Leitweg-ID for a public body.
                </p>
              )}
            </div>
          </SettingsSection>

          <SettingsSection
            title="Invoice footer"
            description="Information appended to the bottom of every invoice, per language."
            className="mt-6"
          >
            <div className="space-y-4">
              <div className="space-y-1.5">
                <div className="flex flex-wrap items-center gap-1.5">
                  <div className="inline-flex flex-wrap items-center gap-1 rounded-lg border border-border bg-muted/40 p-0.5">
                    {footerTabs.map(locale => {
                      const active = locale === shownLang
                      // The default can't go, and a language only pinned because
                      // invoices are issued in it has nothing to remove.
                      const removable =
                        locale !== homeLang &&
                        (addedLangs.includes(locale) ||
                          hasFooterOverride(footerTranslations?.[locale]))
                      return (
                        <div
                          key={locale}
                          className={cn(
                            'flex items-center rounded-md text-[13px] font-medium transition-colors',
                            active
                              ? 'bg-background text-foreground shadow-sm'
                              : 'text-muted-foreground hover:text-foreground'
                          )}
                        >
                          <button
                            type="button"
                            onClick={() => setActiveLang(locale)}
                            className={cn('py-1 pl-3', removable ? 'pr-1' : 'pr-3')}
                          >
                            {footerLocaleLabel(locale)}
                            {locale === homeLang && (
                              <span className="ml-1.5 font-normal text-muted-foreground">
                                default
                              </span>
                            )}
                          </button>
                          {removable && (
                            <button
                              type="button"
                              onClick={() => removeFooterLang(locale)}
                              aria-label={`Remove the ${footerLocaleLabel(locale)} footer`}
                              className="mr-1 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                            >
                              <XIcon size={12} />
                            </button>
                          )}
                        </div>
                      )
                    })}
                  </div>
                  {addableLangs.length > 0 && (
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-7 gap-1 text-[13px]"
                        >
                          <PlusIcon size={13} />
                          Add language
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="start">
                        {addableLangs.map(locale => (
                          <DropdownMenuItem key={locale} onSelect={() => addFooterLang(locale)}>
                            {footerLocaleLabel(locale)}
                          </DropdownMenuItem>
                        ))}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  )}
                </div>
                <p className="text-[13px] text-muted-foreground">
                  {onHomeTab
                    ? 'Printed on invoices in any language that has no footer of its own.'
                    : shownLang === effectiveLang && !hasFooterOverride(shownFooter)
                      ? `Invoices are issued in ${footerLocaleLabel(shownLang)}. Until this is filled in, they print the ${footerLocaleLabel(homeLang)} default.`
                      : `Fields left empty print the ${footerLocaleLabel(homeLang)} default.`}
                </p>
              </div>

              <SettingsFieldGrid>
                <TextareaFormField
                  key={`${shownLang}-info`}
                  name={footerInfoName}
                  control={methods.control}
                  label="Additional information"
                  placeholder={fallbackPlaceholder('info')}
                  containerClassName="sm:col-span-2"
                />
                <TextareaFormField
                  key={`${shownLang}-legal`}
                  name={footerLegalName}
                  label="Legal information"
                  control={methods.control}
                  placeholder={fallbackPlaceholder('legal')}
                  containerClassName="sm:col-span-2"
                />
              </SettingsFieldGrid>
            </div>
          </SettingsSection>
        </form>
      </Form>
    </div>
  )
}
