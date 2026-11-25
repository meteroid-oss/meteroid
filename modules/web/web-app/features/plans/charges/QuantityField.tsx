import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useEffect, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  createManagedQuantity,
  listManagedQuantities,
} from '@/rpc/api/quantities/v1/quantities-ManagedQuantitiesService_connectquery'

import type { ManagedQuantity } from '@/rpc/api/quantities/v1/models_pb'
import type { Control, FieldValues, UseFormSetValue } from 'react-hook-form'

const NEW = '__new'

/** A quantity's code from its name: lowercase letters, digits, `_` and `-`. */
export const quantityCode = (name: string) =>
  name
    .trim()
    .toLowerCase()
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')

/** The tenant's seats-like quantity, else its first: what a new seat charge counts. */
export const defaultQuantity = (quantities: ManagedQuantity[]) =>
  quantities.find(q => !q.archived && (/seat/i.test(q.code) || /seat/i.test(q.name))) ??
  quantities.find(q => !q.archived)

/**
 * What a seat charge counts: one of the tenant's managed quantities, or a new one made here.
 * Charges counting the same quantity share its count. The seats' label follows the quantity.
 */
export const QuantityField = ({
  control,
  setValue,
  quantityId,
}: {
  control: Control<FieldValues>
  setValue: UseFormSetValue<FieldValues>
  quantityId?: string
}) => {
  const quantitiesQuery = useQuery(listManagedQuantities, {})
  const quantities = (quantitiesQuery.data?.quantities ?? []).filter(
    q => !q.archived || q.id === quantityId
  )
  const [creating, setCreating] = useState(false)
  const [name, setName] = useState('')

  const choose = (quantity: ManagedQuantity) => {
    setValue('quantityId', quantity.id, { shouldDirty: true, shouldValidate: true })
    setValue('slotUnitName', quantity.name, { shouldDirty: true })
  }

  // A new seat charge counts the tenant's seats until another quantity is picked.
  useEffect(() => {
    if (quantityId || !quantitiesQuery.data) return
    const fallback = defaultQuantity(quantitiesQuery.data.quantities)
    if (fallback) choose(fallback)
    else setCreating(true)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [quantityId, quantitiesQuery.data])

  const create = useMutation(createManagedQuantity, {
    onSuccess: ({ quantity }) => {
      void quantitiesQuery.refetch()
      if (quantity) choose(quantity)
      setCreating(false)
      setName('')
    },
  })
  const code = quantityCode(name)
  const [error, setError] = useState<string>()
  const submitNew = () => {
    if (!code) return
    setError(undefined)
    create.mutate(
      { code, name: name.trim() },
      { onError: e => setError(rpcErrorMessage(e, 'The quantity could not be created')) }
    )
  }

  return (
    <FormField
      control={control}
      name="quantityId"
      render={({ field }) => (
        <FormItem className="max-w-xs space-y-1.5">
          <FormLabel className="text-xs font-normal text-muted-foreground">Counted in</FormLabel>
          {creating ? (
            <div className="flex items-center gap-2">
              <Input
                autoFocus
                className="h-9"
                placeholder="e.g. Editors"
                value={name}
                onChange={e => setName(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter') {
                    e.preventDefault()
                    submitNew()
                  }
                }}
              />
              <Button
                type="button"
                size="sm"
                variant="secondary"
                disabled={!code || create.isPending}
                onClick={submitNew}
              >
                Create
              </Button>
              {quantities.length > 0 && (
                <Button type="button" size="sm" variant="ghost" onClick={() => setCreating(false)}>
                  Cancel
                </Button>
              )}
            </div>
          ) : (
            <Select
              value={field.value || undefined}
              disabled={quantitiesQuery.isLoading}
              onValueChange={next => {
                if (next === NEW) return setCreating(true)
                const picked = quantities.find(q => q.id === next)
                if (picked) choose(picked)
              }}
            >
              <SelectTrigger className="h-9 text-sm">
                <SelectValue placeholder="Choose a quantity" />
              </SelectTrigger>
              <SelectContent>
                {quantities.map(q => (
                  <SelectItem key={q.id} value={q.id}>
                    {q.name}
                  </SelectItem>
                ))}
                <SelectItem value={NEW}>New quantity…</SelectItem>
              </SelectContent>
            </Select>
          )}
          {error && <p className="text-xs text-destructive">{error}</p>}
          <FormMessage className="text-xs" />
        </FormItem>
      )}
    />
  )
}
