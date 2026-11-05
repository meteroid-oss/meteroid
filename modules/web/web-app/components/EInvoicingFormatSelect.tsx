import { ComboboxFormField } from '@ui/components'
import { Control, FieldPath, FieldValues, useWatch } from 'react-hook-form'

import { EInvoicingFormat } from '@/rpc/api/invoicingentities/v1/models_pb'

/** Form value meaning "PDF only" — the entity has not opted into a standard. */
export const EINVOICING_FORMAT_OFF = 'off'

const BY_NAME: Record<string, EInvoicingFormat> = {
  FACTUR_X: EInvoicingFormat.FACTUR_X,
  XRECHNUNG: EInvoicingFormat.XRECHNUNG,
  PEPPOL_BIS: EInvoicingFormat.PEPPOL_BIS,
}

const LABELS: Record<string, string> = {
  FACTUR_X: 'Factur-X / ZUGFeRD (France, Germany)',
  XRECHNUNG: 'XRechnung (Germany, public sector)',
  PEPPOL_BIS: 'Peppol BIS Billing 3.0',
}

/**
 * Peppol BIS routes by participant identifier, which only a Peppol transport can
 * supply, so selecting it would fail every invoice.
 * `peppol_is_not_satisfiable_without_a_transport` pins the gap.
 */
const SELECTABLE = ['FACTUR_X', 'XRECHNUNG']

const option = (value: string) => ({ label: LABELS[value] ?? value, value })

export const EINVOICING_FORMAT_OPTIONS: { label: string; value: string }[] = [
  { label: 'Off — PDF only', value: EINVOICING_FORMAT_OFF },
  ...SELECTABLE.map(option),
]

/** Stored enum → form value. */
export const einvoicingFormatFromStored = (value?: EInvoicingFormat): string => {
  const found = Object.entries(BY_NAME).find(([, v]) => v === value)
  return found ? found[0] : EINVOICING_FORMAT_OFF
}

/**
 * Form value → patch payload. "Off" sends UNSPECIFIED, which the backend stores
 * as NULL — so turning it off actually turns it off (a bare `undefined` would
 * leave the stored value untouched).
 */
export const einvoicingFormatForPatch = (value?: string): EInvoicingFormat =>
  (value && BY_NAME[value]) || EInvoicingFormat.EINVOICING_FORMAT_UNSPECIFIED

interface Props<TFieldValues extends FieldValues, TName extends FieldPath<TFieldValues>> {
  name: TName
  control: Control<TFieldValues>
  label?: string
  containerClassName?: string
}

export function EInvoicingFormatSelect<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
>({ name, control, label = 'E-invoicing format', containerClassName }: Props<TFieldValues, TName>) {
  const current = useWatch({ control, name })

  // A format set outside the UI still has to read back as itself rather than
  // silently as "Off", which a Save would then write back as a real change.
  const options =
    typeof current === 'string' &&
    current !== EINVOICING_FORMAT_OFF &&
    !SELECTABLE.includes(current)
      ? [...EINVOICING_FORMAT_OPTIONS, option(current)]
      : EINVOICING_FORMAT_OPTIONS

  return (
    <ComboboxFormField
      name={name}
      label={label}
      control={control}
      containerClassName={containerClassName}
      placeholder="Off — PDF only"
      options={options}
    />
  )
}
