import { useTenant } from '@/hooks/useTenant'
import { formatCurrency, formatCurrencyNoRounding } from '@/utils/numbers'

export const useCurrency = () => {
  const { tenant } = useTenant()

  const formatCurrencyWithTenant = (value: number | string | bigint | undefined) => {
    if (!tenant) return '...'
    if (value === undefined) return 'No data'

    if (typeof value === 'bigint') {
      return formatCurrency(value, tenant.reportingCurrency)
    } else {
      return formatCurrencyNoRounding(value, tenant.reportingCurrency)
    }
  }
  return { currency: tenant?.reportingCurrency, formatAmount: formatCurrencyWithTenant }
}
