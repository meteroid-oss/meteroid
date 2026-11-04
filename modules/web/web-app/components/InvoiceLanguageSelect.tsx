import { ComboboxFormField } from '@ui/components'
import { Control, FieldPath, FieldValues } from 'react-hook-form'

/** Sentinel form value meaning "no explicit choice — inherit from the level above". */
export const INVOICE_LANGUAGE_DEFAULT = 'default'

/** Languages the invoice/credit-note renderer supports today. */
export const INVOICE_LANGUAGE_OPTIONS: { label: string; value: string }[] = [
  { label: 'Default (inherit)', value: INVOICE_LANGUAGE_DEFAULT },
  { label: 'English', value: 'en-US' },
  { label: 'Français', value: 'fr-FR' },
  { label: 'Deutsch', value: 'de-DE' },
  { label: 'Español', value: 'es-ES' },
  { label: 'Italiano', value: 'it-IT' },
  { label: 'Nederlands', value: 'nl-NL' },
  { label: 'Português (Portugal)', value: 'pt-PT' },
  { label: 'Português (Brasil)', value: 'pt-BR' },
]

/** Form value → create payload: omit when inheriting so the column stays NULL. */
export const invoiceLanguageForCreate = (value?: string): string | undefined =>
  value && value !== INVOICE_LANGUAGE_DEFAULT ? value : undefined

/**
 * Form value → patch/update payload. "Default" sends an empty string, which the
 * backend stores as NULL — so switching back to Default actually resets to inherit
 * (a bare `undefined` would leave the stored value untouched).
 */
export const invoiceLanguageForPatch = (value?: string): string =>
  value && value !== INVOICE_LANGUAGE_DEFAULT ? value : ''

/** Stored value (null/empty when inheriting) → form value. */
export const invoiceLanguageFromStored = (value?: string): string =>
  value || INVOICE_LANGUAGE_DEFAULT

/**
 * Customers hold an ordered preference list (`preferredLocales`, the shape Stripe uses),
 * the invoicing entity a single default. The picker offers one language either way, so on
 * a customer it edits the first entry and an empty list means "inherit".
 */
export const invoiceLanguageFromLocales = (locales?: string[]): string =>
  invoiceLanguageFromStored(locales?.[0])

export const invoiceLanguageToLocales = (value?: string): string[] =>
  value && value !== INVOICE_LANGUAGE_DEFAULT ? [value] : []

/**
 * Whether leaving this field on "Default (inherit)" lets the country default decide
 * a question the issuer's law answers with nullity. Mirror of
 * `Countries::document_language_is_an_unsafe_guess` in meteroid-store's
 * `constants.rs`, which carries the full reasoning: a Belgian invoice is a document
 * prescribed by law, so Belgium's language legislation binds it to the issuer's
 * *exploitatiezetel* — Dutch in Flanders, French in Wallonia, German in the
 * German-speaking Community, either in Brussels — and a breaching document is void
 * (*nietig*), raised by the court of its own motion. The country default (`nl-BE`)
 * is right for Flanders and void in Wallonia, so an unconfigured Belgian entity is
 * guessing. Only the operator knows where the entity is established; surface the
 * question here, where they can still answer it.
 */
export const invoiceLanguageIsUnsafeGuess = (
  formValue: string | undefined,
  entityCountry: string | undefined
): boolean => entityCountry === 'BE' && (!formValue || formValue === INVOICE_LANGUAGE_DEFAULT)

interface Props<TFieldValues extends FieldValues, TName extends FieldPath<TFieldValues>> {
  name: TName
  control: Control<TFieldValues>
  label?: string
  placeholder?: string
  className?: string
  containerClassName?: string
  labelClassName?: string
  layout?: 'vertical' | 'horizontal' | null
}

export function InvoiceLanguageSelect<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
>({
  name,
  control,
  label = 'Invoice language',
  placeholder = 'Default (inherit)',
  className,
  containerClassName,
  labelClassName,
  layout = 'vertical',
}: Props<TFieldValues, TName>) {
  return (
    <ComboboxFormField
      name={name}
      label={label}
      control={control}
      className={className}
      containerClassName={containerClassName}
      labelClassName={labelClassName}
      layout={layout}
      placeholder={placeholder}
      options={INVOICE_LANGUAGE_OPTIONS}
    />
  )
}

/**
 * Countries where a single default language is a poor guess, so the operator should be
 * asked at creation instead of inheriting one. Mirrors
 * `Countries::document_language_deserves_an_explicit_choice` in `constants.rs` — keep the
 * two in step.
 *
 * Two different reasons land here. Belgium's language law *binds* the issuer by seat of
 * establishment and a wrong-language invoice can be void, so the answer must be right.
 * Switzerland binds nothing — neither the MWSTG nor the MWSTV says anything about the
 * language of an invoice — but it has four official languages and `de-CH` serves only
 * the 62% who speak German, so the answer is merely usually wrong.
 */
const LANGUAGE_DESERVES_EXPLICIT_CHOICE = ['BE', 'CH']

export const invoiceLanguageDeservesExplicitChoice = (entityCountry?: string): boolean =>
  !!entityCountry && LANGUAGE_DESERVES_EXPLICIT_CHOICE.includes(entityCountry)
