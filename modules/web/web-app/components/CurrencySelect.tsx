import { useQuery } from '@/lib/connectrpc'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import { SelectFormField, SelectItem } from '@ui/components'
import { Control, FieldPath, FieldValues } from 'react-hook-form'

interface Props<TFieldValues extends FieldValues, TName extends FieldPath<TFieldValues>> {
  name: TName
  control: Control<TFieldValues>
  required?: boolean
  label?: string
  placeholder?: string
  className?: string
  layout?: 'horizontal' | 'vertical'
}

export function CurrencySelect<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
>({
  name,
  control,
  required,
  placeholder = 'Currency',
  label = 'Currency',
  layout = 'horizontal',
  className,
}: Props<TFieldValues, TName>) {
  const activeCurrenciesQuery = useQuery(listTenantCurrencies)
  const activeCurrencies = activeCurrenciesQuery.data?.currencies ?? []

  // TODO if no value set default to accounting currency ?

  return (
    <SelectFormField
      name={name}
      label={label}
      layout={layout}
      required={required}
      placeholder={placeholder}
      className={className}
      empty={activeCurrencies.length === 0}
      control={control}
    >
      {activeCurrencies.map((a, i) => (
        <SelectItem value={a} key={`item` + i}>
          {a}
        </SelectItem>
      ))}
    </SelectFormField>
  )
}
