import { ComboboxFormField } from '@ui/components'
import { Control, FieldPath, FieldValues } from 'react-hook-form'

import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { useQuery } from '@/lib/connectrpc'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

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

export function CountrySelect<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
>({
  name,
  control,
  placeholder = 'Select a country',
  label = 'Country',
  className,
  containerClassName,
  labelClassName,
  layout = 'vertical',
}: Props<TFieldValues, TName>) {
  const getCountriesQuery = useQuery(getCountries)

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
      hasSearch
      options={
        getCountriesQuery.data?.countries.map(country => ({
          label: (
            <span className="flex flex-row">
              <span className="pr-2">{getCountryFlagEmoji(country.code)}</span>
              <span>{getCountryName(country.code)}</span>
            </span>
          ),
          value: country.code,
          keywords: [getCountryName(country.code), country.code],
        })) ?? []
      }
    />
  )
}
