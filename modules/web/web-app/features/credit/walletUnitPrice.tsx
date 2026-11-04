import { skipToken } from '@connectrpc/connect-query'
import { Alert, AlertDescription, InputFormField } from '@md/ui'

import { unitPriceSchema } from '@/features/credit/unitSale'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { getCreditUnit } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'

import type { AgreedUnitPrice, Wallet } from '@/rpc/api/wallets/v1/models_pb'
import type { Control, FieldPath, FieldValues } from 'react-hook-form'

/** Why an agreed price typed in the settings is refused; blank sells at the list price. */
export const unitPriceIssue = (value: string): string | undefined => {
  if (!value.trim()) return undefined
  const price = unitPriceSchema.safeParse(value)
  return price.success ? undefined : price.error.issues[0]?.message
}

/**
 * How a credit unit wallet is sold to its customer: the unit's list price in the customer's
 * currency and the price agreed with them, which applies only in the currency it was set in.
 */
export const useWalletUnitPricing = (wallet: Wallet, customerCurrency: string) => {
  const creditUnit = wallet.creditUnit
  const sameCurrency = (currency: string) => currency.toUpperCase() === customerCurrency.toUpperCase()
  const agreedPrice =
    creditUnit && wallet.unitPrice && sameCurrency(wallet.unitPrice.currency)
      ? wallet.unitPrice.unitPrice
      : undefined
  // Saving clears an agreed price set in another currency.
  const staleAgreedPrice: AgreedUnitPrice | undefined =
    creditUnit && wallet.unitPrice && !agreedPrice ? wallet.unitPrice : undefined

  const unitQuery = useQuery(getCreditUnit, creditUnit ? { id: creditUnit.id } : skipToken)
  const unit = unitQuery.data?.unit
  const saleBlocker =
    creditUnit && unit
      ? !unit.productId
        ? `${creditUnit.pluralName} cannot be sold until a tax product is set in Settings, Credit units.`
        : !unit.prices.some(price => sameCurrency(price.currency))
          ? `${creditUnit.pluralName} have no selling price in ${customerCurrency}. Set one in Settings, Credit units.`
          : undefined
      : undefined
  const sellable = !creditUnit || (!!unit && !saleBlocker)
  const listPrice = unit?.prices.find(price => sameCurrency(price.currency))?.unitPrice

  return { unit, saleBlocker, sellable, listPrice, agreedPrice, staleAgreedPrice }
}

/** The agreed price per credit of a credit unit wallet, as a section of its settings form. */
export const WalletUnitPriceSection = <T extends FieldValues & { unitPrice: string }>({
  control,
  customerCurrency,
  listPrice,
  staleAgreedPrice,
  description,
  divided = true,
}: {
  control: Control<T>
  customerCurrency: string
  listPrice?: string
  staleAgreedPrice?: AgreedUnitPrice
  description: string
  divided?: boolean
}) => (
  <SettingsSection title="Price" divided={divided}>
    <div className="space-y-4">
      {staleAgreedPrice && (
        <Alert variant="warning">
          <AlertDescription className="text-xs">
            The agreed price of{' '}
            {formatCurrencyNoRounding(staleAgreedPrice.unitPrice, staleAgreedPrice.currency)} was
            set in {staleAgreedPrice.currency} and does not apply in {customerCurrency}. Saving
            removes it.
          </AlertDescription>
        </Alert>
      )}
      <InputFormField
        control={control}
        name={'unitPrice' as FieldPath<T>}
        label={`Agreed price per credit (optional, ${customerCurrency})`}
        placeholder={listPrice}
        inputMode="decimal"
        description={description}
      />
    </div>
  </SettingsSection>
)
