import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { InputFormField, SelectFormField, SelectItem, TextareaFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { CatalogEditPanel } from '@/features/productCatalog/generic/CatalogEditPanel'
import { ProductAppearancePicker } from '@/features/productCatalog/items/ProductAppearancePicker'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'
import { createProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { listTaxCategories } from '@/rpc/api/taxes/v1/taxes-TaxesService_connectquery'

import type { ProductAppearance } from '@/features/productCatalog/items/ProductIcon'

const ENTITY_DEFAULT_CATEGORY = '__entity_default__'

// A product is what is sold: how it is billed is each charge's, set where it is priced.
const ProductCreateSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  description: z.string().max(2048).optional(),
  familyLocalId: z.string().min(1, 'Product line is required'),
  taxCategoryId: z.string(),
})

interface ProductCreatePanelProps {
  visible: boolean
  onClose: () => void
  onCreated?: (productId: string) => void
}

export const ProductCreatePanel = ({ visible, onClose, onCreated }: ProductCreatePanelProps) => {
  const queryClient = useQueryClient()

  const familiesQuery = useQuery(listProductFamilies)
  const families = familiesQuery.data?.productFamilies ?? []
  const taxCategoriesQuery = useQuery(listTaxCategories, {})
  const [appearance, setAppearance] = useState<ProductAppearance>({})
  useEffect(() => {
    if (!visible) setAppearance({})
  }, [visible])

  const methods = useZodForm({
    schema: ProductCreateSchema,
    defaultValues: {
      name: '',
      description: '',
      familyLocalId: '',
      taxCategoryId: ENTITY_DEFAULT_CATEGORY,
    },
  })

  const defaultFamilyLocalId = families[0]?.localId
  const { getValues, setValue } = methods
  useEffect(() => {
    if (!getValues('familyLocalId') && defaultFamilyLocalId) {
      setValue('familyLocalId', defaultFamilyLocalId)
    }
  }, [defaultFamilyLocalId, getValues, setValue])

  const createProductMut = useMutation(createProduct, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: createProduct.parent, cardinality: undefined }),
      })
    },
  })

  const onSubmit = async (values: z.infer<typeof ProductCreateSchema>) => {
    try {
      const res = await createProductMut.mutateAsync({
        name: values.name,
        description: values.description || undefined,
        familyLocalId: values.familyLocalId,
        catalog: true,
        taxCategoryId:
          values.taxCategoryId === ENTITY_DEFAULT_CATEGORY ? undefined : values.taxCategoryId,
        icon: appearance.icon,
        color: appearance.color,
      })
      toast.success('Product created')
      if (res.product) onCreated?.(res.product.id)
    } catch (e) {
      toast.error(`Failed to create product: ${(e as Error).message}`)
      throw e
    }
  }

  return (
    <CatalogEditPanel
      visible={visible}
      closePanel={onClose}
      title="New product"
      methods={methods}
      onSubmit={onSubmit}
    >
      <div className="flex flex-col gap-4 mt-4">
        <div className="flex items-end gap-2">
          <div className="pb-1">
            <ProductAppearancePicker {...appearance} onChange={setAppearance} />
          </div>
          <InputFormField
            name="name"
            label="Name"
            control={methods.control}
            containerClassName="flex-1"
          />
        </div>
        <TextareaFormField
          name="description"
          label="Description (optional)"
          control={methods.control}
        />
        {families.length > 1 && (
          <SelectFormField name="familyLocalId" label="Product line" control={methods.control}>
            {families.map(f => (
              <SelectItem key={f.localId} value={f.localId}>
                {f.name}
              </SelectItem>
            ))}
          </SelectFormField>
        )}
        <SelectFormField name="taxCategoryId" label="Tax category" control={methods.control}>
          <SelectItem value={ENTITY_DEFAULT_CATEGORY}>Entity default</SelectItem>
          {(taxCategoriesQuery.data?.taxCategories ?? []).map(c => (
            <SelectItem key={c.id} value={c.id}>
              {c.name}
            </SelectItem>
          ))}
        </SelectFormField>
        <p className="text-xs text-muted-foreground">
          How it is billed and its prices are set when it is added to a plan or add-on: each
          charge may bill it its own way.
        </p>
      </div>
    </CatalogEditPanel>
  )
}
