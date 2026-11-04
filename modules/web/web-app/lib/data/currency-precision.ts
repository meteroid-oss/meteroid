// Currency minor-unit precision, sourced from the backend `get_currencies` RPC — the single
// source of truth (the `meteroid_money` ISO table). The map is replaced when the currency list
// loads (see TenantLayout); an unknown code falls back to 2.
//
// It is seeded with the non-2-decimal currencies so money formatting AND money input
// (`majorToMinorUnit`) are correct even before the RPC resolves — otherwise a 0-decimal amount
// (e.g. ¥15000) entered during that window would be scaled 100× wrong. Only the exceptions to
// the default of 2 need listing; the backend map overwrites all of this once loaded.
// Must stay in sync with the non-2-decimal currencies in the backend `meteroid_money` ISO
// table (the authoritative source). IQD is deliberately 2 there (kept at 2 to match existing
// invoices), so it is NOT listed. Non-billable codes (precious metals, test/bond X-codes,
// defunct currencies) are omitted — they can never be an invoice currency.
const NON_DEFAULT_PRECISION: Readonly<Record<string, number>> = {
  // 0-decimal
  BIF: 0, CLP: 0, DJF: 0, GNF: 0, ISK: 0, JPY: 0, KMF: 0, KRW: 0, PYG: 0,
  RWF: 0, UGX: 0, VND: 0, VUV: 0, XAF: 0, XOF: 0, XPF: 0,
  // 3-decimal
  BHD: 3, JOD: 3, KWD: 3, LYD: 3, OMR: 3, TND: 3,
  // 4-decimal
  CLF: 4, UYW: 4,
}

let precisionByCode: Record<string, number> = { ...NON_DEFAULT_PRECISION }

export const setCurrencyPrecisions = (
  currencies: readonly { code: string; precision: number }[]
): void => {
  precisionByCode = Object.fromEntries(currencies.map(c => [c.code, c.precision]))
}

// Credit unit codes (4+ characters, never an ISO code) key wallets, budgets and ledger amounts
// like a currency; their precision comes from the tenant's credit unit catalog.
let creditUnitPrecisionByCode: Record<string, number> = {}

export const setCreditUnitPrecisions = (
  units: readonly { code: string; precision: number }[]
): void => {
  creditUnitPrecisionByCode = Object.fromEntries(units.map(u => [u.code, u.precision]))
}

export const currencyPrecision = (currencyCode: string): number =>
  precisionByCode[currencyCode] ?? creditUnitPrecisionByCode[currencyCode] ?? 2
