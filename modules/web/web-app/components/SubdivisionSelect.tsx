import { ComboboxFormField } from '@ui/components'
import { Control, FieldPath, FieldValues, useWatch } from 'react-hook-form'

import { useQuery } from '@/lib/connectrpc'
import { getSubdivisions } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

interface Props<TFieldValues extends FieldValues, TName extends FieldPath<TFieldValues>> {
  name: TName
  control: Control<TFieldValues>
  countryFieldName: FieldPath<TFieldValues>
  required?: boolean
  label?: string
  placeholder?: string
  className?: string
}

export function SubdivisionSelect<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
>({
  name,
  control,
  countryFieldName,
  required,
  placeholder = 'Select a subdivision',
  label = 'Subdivision',
  className,
}: Props<TFieldValues, TName>) {
  const countryCode = useWatch({
    name: countryFieldName,
    control,
  })

  const getSubdivisionsQuery = useQuery(
    getSubdivisions,
    { countryCode: countryCode || '' },
    {
      enabled: !!countryCode,
    }
  )

  const subdivisions = getSubdivisionsQuery.data?.subdivisions || []

  return (
    <ComboboxFormField
      name={name}
      label={label}
      control={control}
      className={className}
      placeholder={placeholder}
      hasSearch
      required={required}
      disabled={!countryCode || subdivisions.length === 0}
      options={subdivisions.map(subdivision => ({
        label: subdivision.name,
        value: subdivision.code,
        keywords: [subdivision.name, subdivision.code],
      }))}
    />
  )
}
