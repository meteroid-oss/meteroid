import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from '@md/ui'
import { useState } from 'react'

import { ResourceTypeSheet } from '@/features/resources/ResourceTypeSheet'
import { useResourceTypes } from '@/features/resources/hooks'

const CUSTOMER = '__customer__'
const DEFINE = '__define__'

/** Customer or a resource type, with "New resource type…" to create one in context. */
export const TypeSelect = ({
  value,
  onChange,
  disabled,
  customerLabel = 'Customer',
}: {
  value: string | undefined
  onChange: (typeId: string | undefined) => void
  disabled?: boolean
  customerLabel?: string
}) => {
  const { types } = useResourceTypes()
  const [defining, setDefining] = useState(false)
  return (
    <>
      <Select
        value={value || CUSTOMER}
        disabled={disabled}
        onValueChange={v => (v === DEFINE ? setDefining(true) : onChange(v === CUSTOMER ? undefined : v))}
      >
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={CUSTOMER}>{customerLabel}</SelectItem>
          {types.map(t => (
            <SelectItem key={t.id} value={t.id}>
              {t.name}
            </SelectItem>
          ))}
          <SelectSeparator />
          <SelectItem value={DEFINE}>New resource type…</SelectItem>
        </SelectContent>
      </Select>
      {defining && (
        <ResourceTypeSheet
          onClose={created => {
            setDefining(false)
            if (created) onChange(created.id)
          }}
        />
      )}
    </>
  )
}

const ANY = '__any__'

/** A list filter: any level (undefined), the customer (''), or one type. Hidden without types. */
export const TypeFilter = ({
  value,
  onChange,
  anyLabel,
}: {
  value: string | undefined
  onChange: (typeId: string | undefined) => void
  anyLabel: string
}) => {
  const { all, hasTypes } = useResourceTypes()
  if (!hasTypes) return null
  return (
    <Select value={value === undefined ? ANY : value || CUSTOMER} onValueChange={v => onChange(v === ANY ? undefined : v === CUSTOMER ? '' : v)}>
      <SelectTrigger className="h-8 w-40 text-sm">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ANY}>{anyLabel}</SelectItem>
        <SelectItem value={CUSTOMER}>Customer</SelectItem>
        {all.map(t => (
          <SelectItem key={t.id} value={t.id}>
            {t.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
