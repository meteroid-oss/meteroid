import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Label,
  MultiSelect,
  MultiSelectContent,
  MultiSelectEmpty,
  MultiSelectItem,
  MultiSelectList,
  MultiSelectSearch,
  MultiSelectTrigger,
  MultiSelectValue,
  RadioGroup,
  RadioGroupItem,
} from '@md/ui'
import { useEffect, useMemo, useState } from 'react'
import { useWatch, type Control, type FieldValues } from 'react-hook-form'
import { z } from 'zod'

import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { searchProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

export const productScopeSchema = z.object({
  mode: z.enum(['all', 'selected']),
  productIds: z.array(z.string()),
})

export type ProductScopeValues = z.infer<typeof productScopeSchema>

export const defaultProductScope: ProductScopeValues = { mode: 'all', productIds: [] }

/** Call from the form's `superRefine` wherever the field is shown. */
export const refineProductScope = (
  scope: ProductScopeValues,
  ctx: z.RefinementCtx,
  path: (string | number)[] = ['productScope']
) => {
  if (scope.mode === 'selected' && scope.productIds.length === 0) {
    ctx.addIssue({
      code: 'custom',
      path: [...path, 'productIds'],
      message: 'Choose at least one product',
    })
  }
}

/** The `applicableProductIds` of a grant request: empty applies to every product. */
export const applicableProductIds = (scope: ProductScopeValues): string[] =>
  scope.mode === 'selected' ? scope.productIds : []

type ScopeForm = { productScope: ProductScopeValues }

const OPTIONS = [
  { value: 'all', label: 'All products', hint: "Any product's charges" },
  { value: 'selected', label: 'Selected products', hint: 'Only the products you choose' },
] as const

/** "Applies to" all products or selected ones, bound to the form's `productScope` value. */
export const ProductScopeField = <T extends FieldValues & ScopeForm>({
  control,
  allowedProductIds,
  allowedHint,
}: {
  control: Control<T>
  /** Only these products can be chosen, e.g. the ones a subscription bills. */
  allowedProductIds?: ReadonlySet<string>
  allowedHint?: string
}) => {
  const scopeControl = control as unknown as Control<ScopeForm>
  const mode = useWatch({ control: scopeControl, name: 'productScope.mode' })

  return (
    <div className="space-y-3">
      <FormField
        control={scopeControl}
        name="productScope.mode"
        render={({ field }) => (
          <FormItem className="space-y-2">
            <FormLabel>Applies to</FormLabel>
            <FormControl>
              <RadioGroup
                value={field.value}
                onValueChange={field.onChange}
                className="grid gap-2 sm:grid-cols-2"
              >
                {OPTIONS.map(option => {
                  const id = `product-scope-${option.value}`
                  return (
                    <Label
                      key={option.value}
                      htmlFor={id}
                      className="flex cursor-pointer items-start gap-2 rounded-md border border-border p-3 font-normal has-[[data-state=checked]]:border-primary"
                    >
                      <RadioGroupItem value={option.value} id={id} className="mt-0.5" />
                      <span className="space-y-0.5">
                        <span className="block text-sm font-medium">{option.label}</span>
                        <span className="block text-xs text-muted-foreground">{option.hint}</span>
                      </span>
                    </Label>
                  )
                })}
              </RadioGroup>
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
      {mode === 'selected' && (
        <FormField
          control={scopeControl}
          name="productScope.productIds"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Products</FormLabel>
              <FormControl>
                <ProductMultiSelect
                  value={field.value}
                  onChange={field.onChange}
                  allowedProductIds={allowedProductIds}
                />
              </FormControl>
              {allowedHint && <p className="text-xs text-muted-foreground">{allowedHint}</p>}
              <FormMessage />
            </FormItem>
          )}
        />
      )}
    </div>
  )
}

const ProductMultiSelect = ({
  value,
  onChange,
  allowedProductIds,
}: {
  value: string[]
  onChange: (ids: string[]) => void
  allowedProductIds?: ReadonlySet<string>
}) => {
  const [search, setSearch] = useState<string | undefined>(undefined)
  const query = useDebounceValue(search?.trim() || undefined, 250)
  const list = useQuery(searchProducts, {
    query,
    pagination: { perPage: 20, page: 0 },
  })
  const results = useMemo(
    () =>
      (list.data?.products ?? []).filter(
        product => !allowedProductIds || allowedProductIds.has(product.id)
      ),
    [list.data?.products, allowedProductIds]
  )

  // Names of products seen in any search, so a selection stays legible once the search moves on.
  const [names, setNames] = useState<Record<string, string>>({})
  useEffect(() => {
    if (results.length === 0) return
    setNames(previous => {
      const next = { ...previous }
      for (const product of results) next[product.id] = product.name
      return next
    })
  }, [results])

  const optionIds = useMemo(() => {
    const resultIds = results.map(product => product.id)
    const seen = new Set(resultIds)
    return [...resultIds, ...value.filter(id => !seen.has(id))]
  }, [results, value])

  return (
    <MultiSelect value={value} onValueChange={onChange} onSearch={setSearch}>
      <MultiSelectTrigger>
        <MultiSelectValue placeholder="Choose products" maxDisplay={3} maxItemLength={24} />
      </MultiSelectTrigger>
      <MultiSelectContent>
        <MultiSelectSearch placeholder="Search products…" />
        <MultiSelectList>
          <MultiSelectEmpty>No products found</MultiSelectEmpty>
          {optionIds.map(id => (
            <MultiSelectItem key={id} value={id} label={names[id] ?? id}>
              {names[id] ?? id}
            </MultiSelectItem>
          ))}
        </MultiSelectList>
      </MultiSelectContent>
    </MultiSelect>
  )
}
