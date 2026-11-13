export interface CsvColumnSpec {
  name: string
  hint?: string
  /** Leaf headers of a `group.*` column. */
  fields?: string[]
}

const group = (prefix: string, fields: string[], hint?: string): CsvColumnSpec => ({
  name: `${prefix}.*`,
  fields: fields.map(field => `${prefix}.${field}`),
  hint,
})

const addressFields = ['line1', 'line2', 'city', 'country', 'state', 'zip_code']
const taxRateFields = ['tax_code', 'name', 'rate']

export const customerCsvRequiredColumns: CsvColumnSpec[] = [
  { name: 'name', hint: 'Company name. Leave blank for INDIVIDUAL customers' },
  { name: 'currency', hint: '3-letter ISO code (e.g., USD, EUR)' },
]

export const customerCsvOptionalColumns: CsvColumnSpec[] = [
  { name: 'invoicing_emails', hint: 'Comma-separated for multiple emails' },
  { name: 'alias' },
  { name: 'billing_email' },
  { name: 'phone' },
  { name: 'customer_type', hint: 'COMPANY (default) or INDIVIDUAL' },
  { name: 'first_name', hint: 'Required for INDIVIDUAL customers' },
  { name: 'last_name', hint: 'Required for INDIVIDUAL customers' },
  { name: 'invoicing_entity_id', hint: '`default` entity is used if empty' },
  { name: 'vat_number' },
  { name: 'legal_number', hint: 'National registration number (e.g., SIREN/SIRET, HRB)' },
  { name: 'buyer_reference', hint: 'Reference the buyer routes invoices by (e.g., Leitweg-ID)' },
  { name: 'is_tax_exempt', hint: 'true or false' },
  { name: 'exemption_reason' },
  {
    name: 'preferred_locales',
    hint: 'Comma-separated BCP-47 codes, most-preferred first (e.g., fr-FR,en); overrides the invoicing entity default',
  },
  { name: 'invoicing_language', hint: 'Deprecated, use preferred_locales' },
  group('tax_rate1', taxRateFields),
  group('tax_rate2', taxRateFields),
  group('billing_address', addressFields),
  group(
    'shipping_address',
    ['same_as_billing', ...addressFields],
    'same_as_billing: true or false'
  ),
]

export const csvHeaders = (columns: CsvColumnSpec[]): string[] =>
  columns.flatMap(column => column.fields ?? [column.name])
