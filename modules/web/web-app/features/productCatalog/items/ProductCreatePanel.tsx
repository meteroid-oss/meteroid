import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { InputFormField, SelectFormField, SelectItem, TextareaFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { EditableStructuralFields } from '@/features/plans/pricecomponents/ProductPricingForm'
import { feeTypeToHuman } from '@/features/plans/pricecomponents/utils'
import { buildFeeStructure, feeTypeToProto } from '@/features/pricing/conversions'
import { CatalogEditPanel } from '@/features/productCatalog/generic/CatalogEditPanel'
import { useZodForm, type Methods } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'
import { createProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { listTaxCategories } from '@/rpc/api/taxes/v1/taxes-TaxesService_connectquery'

import type { ComponentFeeType } from '@/features/pricing/conversions'

const FEE_TYPES = [
  'rate',
  'slot',
  'capacity',
  'usage',
  'extraRecurring',
  'oneTime',
  'credit',
] as const

const ENTITY_DEFAULT_CATEGORY = '__entity_default__'

const ProductCreateSchema = z
  .object({
    name: z.string().min(1, 'Name is required'),
    description: z.string().max(2048).optional(),
    familyLocalId: z.string().min(1, 'Product line is required'),
    feeType: z.enum(FEE_TYPES),
    slotUnitName: z.string().optional(),
    metricId: z.string().optional(),
    usageModel: z.string().optional(),
    billingType: z.string().optional(),
    taxCategoryId: z.string(),
  })
  .superRefine((v, ctx) => {
    if ((v.feeType === 'capacity' || v.feeType === 'usage') && !v.metricId) {
      ctx.addIssue({ code: 'custom', path: ['metricId'], message: 'Metric is required' })
    }
    if (v.feeType === 'slot' && !v.slotUnitName) {
      ctx.addIssue({ code: 'custom', path: ['slotUnitName'], message: 'Unit name is required' })
    }
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

  const methods = useZodForm({
    schema: ProductCreateSchema,
    defaultValues: {
      name: '',
      description: '',
      familyLocalId: '',
      feeType: 'rate',
      slotUnitName: 'Seats',
      metricId: '',
      usageModel: 'per_unit',
      billingType: 'ADVANCE',
      taxCategoryId: ENTITY_DEFAULT_CATEGORY,
    },
  })

  const feeType = methods.watch('feeType') as ComponentFeeType
  const familyLocalId = methods.watch('familyLocalId')

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
        feeType: feeTypeToProto(values.feeType),
        feeStructure: buildFeeStructure(values.feeType, values),
        catalog: true,
        taxCategoryId:
          values.taxCategoryId === ENTITY_DEFAULT_CATEGORY ? undefined : values.taxCategoryId,
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
        <InputFormField name="name" label="Name" control={methods.control} />
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
        <SelectFormField name="feeType" label="Fee type" control={methods.control}>
          {FEE_TYPES.map(ft => (
            <SelectItem key={ft} value={ft}>
              {feeTypeToHuman(ft)}
            </SelectItem>
          ))}
        </SelectFormField>
        <EditableStructuralFields
          feeType={feeType}
          methods={methods as unknown as Methods<z.ZodType>}
          familyLocalId={familyLocalId || undefined}
        />
        <SelectFormField name="taxCategoryId" label="Tax category" control={methods.control}>
          <SelectItem value={ENTITY_DEFAULT_CATEGORY}>Entity default</SelectItem>
          {(taxCategoriesQuery.data?.taxCategories ?? []).map(c => (
            <SelectItem key={c.id} value={c.id}>
              {c.name}
            </SelectItem>
          ))}
        </SelectFormField>
        <p className="text-xs text-muted-foreground">
          Prices are set when the product is added to a plan or add-on.
        </p>
      </div>
    </CatalogEditPanel>
  )
}
