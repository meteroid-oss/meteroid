import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  disableQuery,
  useMutation,
} from '@connectrpc/connect-query'
import { Form } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useSetAtom } from 'jotai'
import { useHydrateAtoms } from 'jotai/utils'
import { ScopeProvider } from 'jotai-scope'
import { ReactNode, useMemo } from 'react'
import { z } from 'zod'

import type { ComponentFeeType } from '@/features/pricing/conversions'
import {
  buildPriceInputs,
  wrapAsNewPriceEntries,
  toPricingTypeFromFeeType,
} from '@/features/pricing/conversions'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { EditPriceComponentCard } from '@/features/plans/pricecomponents/EditPriceComponentCard'
import { extractStructuralInfo } from '@/features/plans/pricecomponents/ProductBrowser'
import {
  PriceComponentFormContent,
  buildDefaultsFromPrices,
  getComponentSchema,
} from '@/features/plans/pricecomponents/ProductPricingForm'
import { editedComponentsAtom, useCurrency } from '@/features/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { useZodForm } from '@/hooks/useZodForm'
import type { PriceComponent as ProtoPriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import {
  editPriceComponent as editPriceComponentMutation,
  listPriceComponents as listPriceComponentsQuery,
} from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { getProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import { componentFeeTypeAtom, componentNameAtom } from './atoms'

// --- Helpers ---

function deriveFeeType(component: ProtoPriceComponent): ComponentFeeType {
  if (component.prices.length > 0) {
    const pricing = component.prices[0].pricing
    switch (pricing.case) {
      case 'ratePricing':
        return 'rate'
      case 'slotPricing':
        return 'slot'
      case 'capacityPricing':
        return 'capacity'
      case 'usagePricing':
        return 'usage'
      case 'extraRecurringPricing':
        return 'extraRecurring'
      case 'oneTimePricing':
        return 'oneTime'
    }
  }
  return 'rate'
}

// --- Edit component ---

interface EditPriceComponentProps {
  component: ProtoPriceComponent
}

export const EditPriceComponent = ({ component }: EditPriceComponentProps) => {
  const setEditedComponents = useSetAtom(editedComponentsAtom)
  const { version } = usePlanWithVersion()
  const queryClient = useQueryClient()
  const currency = useCurrency()

  const feeType = deriveFeeType(component)
  const hasProduct = !!component.productId

  // Fetch the product to get structural info (slot unit name, metric, billing type, etc.)
  const productQuery = useQuery(
    getProduct,
    component.productId ? { productId: component.productId } : disableQuery
  )
  const product = productQuery.data?.product

  const structural = useMemo(
    () => (product ? extractStructuralInfo(feeType, product.feeStructure) : undefined),
    [feeType, product]
  )

  // Derive usage model from existing prices (available synchronously) for schema selection
  const usageModelFromPrices = useMemo(() => {
    if (feeType !== 'usage' || component.prices.length === 0) return undefined
    const pricing = component.prices[0].pricing
    if (pricing.case !== 'usagePricing') return undefined
    const protoToForm: Record<string, string> = {
      perUnit: 'per_unit', tiered: 'tiered', volume: 'volume', package: 'package', matrix: 'matrix',
    }
    return protoToForm[pricing.value.model.case ?? ''] ?? 'per_unit'
  }, [feeType, component.prices])

  const defaultValues = useMemo(
    () => buildDefaultsFromPrices(feeType, component.prices),
    [feeType, component.prices]
  )

  const schema = useMemo(
    () => getComponentSchema(feeType, hasProduct ? 'pricingOnly' : 'full', usageModelFromPrices),
    [feeType, hasProduct, usageModelFromPrices]
  )

  const methods = useZodForm({ schema: schema as z.ZodType, defaultValues })

  const editPriceComponent = useMutation(editPriceComponentMutation, {
    onSuccess: data => {
      if (!version?.id) return
      setEditedComponents(components => components.filter(compId => compId !== component.id))

      if (data.component) {
        queryClient.setQueryData(
          createConnectQueryKey(listPriceComponentsQuery, {
            planVersionId: version.id,
          }),
          createProtobufSafeUpdater(listPriceComponentsQuery, prev => {
            const idx = prev?.components?.findIndex(comp => comp.id === component.id) ?? -1
            if (idx === -1 || !data.component) return prev
            const updated = [...(prev?.components ?? [])]
            updated[idx] = data.component
            return { components: updated }
          })
        )
      }
    },
  })

  const cancel = () => {
    setEditedComponents(components => components.filter(comp => comp !== component.id))
  }

  const onSubmit = () => {
    methods.handleSubmit(
      (formData) => {
        if (!version?.id) return

        const pricingType = toPricingTypeFromFeeType(
          feeType,
          feeType === 'usage' ? (structural?.usageModel ?? usageModelFromPrices ?? (formData as Record<string, unknown>).usageModel as string) : undefined
        )
        const priceInputs = buildPriceInputs(pricingType, formData as Record<string, unknown>, currency)

        editPriceComponent.mutate({
          planVersionId: version.id,
          component: {
            id: component.id,
            name: component.name,
            productId: component.productId,
          },
          prices: wrapAsNewPriceEntries(priceInputs),
        })
      },
      errors => {
        console.error('Edit form validation errors:', errors)
        console.error('Current form values:', methods.getValues())
      }
    )()
  }

  return (
    <ProviderWrapper name={component.name} feeType={feeType}>
      <Form {...methods}>
        <EditPriceComponentCard cancel={cancel} submit={onSubmit}>
          <PriceComponentFormContent
            feeType={feeType}
            currency={currency}
            methods={methods}
            structural={structural}
            editableStructure={!hasProduct}
            isEdit
          />
        </EditPriceComponentCard>
      </Form>
    </ProviderWrapper>
  )
}

// --- Provider wrapper ---

const ProviderWrapper = ({
  children,
  name,
  feeType,
}: {
  children: ReactNode
  name: string
  feeType: ComponentFeeType
}) => {
  return (
    <ScopeProvider atoms={[componentNameAtom, componentFeeTypeAtom]}>
      <HydrateAtoms name={name} feeType={feeType}>
        {children}
      </HydrateAtoms>
    </ScopeProvider>
  )
}

const HydrateAtoms = ({
  name,
  feeType,
  children,
}: {
  name: string
  feeType: ComponentFeeType
  children: ReactNode
}) => {
  useHydrateAtoms([
    [componentNameAtom, name],
    [componentFeeTypeAtom, feeType],
  ])
  return children
}
