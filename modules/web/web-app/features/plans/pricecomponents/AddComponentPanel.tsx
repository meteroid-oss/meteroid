import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  ScrollArea,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { CustomCreationFlow, IdentitySchema } from '@/features/addons/CustomCreationFlow'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { useCurrency } from '@/features/plans/pricecomponents/utils'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing/conversions'
import { useZodForm } from '@/hooks/useZodForm'
import {
  createPriceComponent,
  listPriceComponents,
} from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

import { ProductBrowser } from './ProductBrowser'

import type { ComponentFeeType } from '@/features/pricing/conversions'

export const AddComponentPanel = () => {
  const navigate = useNavigate()
  const { version } = usePlanWithVersion()
  const queryClient = useQueryClient()
  const currency = useCurrency()

  // Custom tab flow state
  const [customStep, setCustomStep] = useState<'identity' | 'feeType' | 'form'>('identity')
  const [customName, setCustomName] = useState('')
  const [customDescription, setCustomDescription] = useState('')
  const [selectedFeeType, setSelectedFeeType] = useState<ComponentFeeType | null>(null)

  const identityMethods = useZodForm({
    schema: IdentitySchema,
    defaultValues: { productName: '', description: '' },
  })

  const createComponent = useMutation(createPriceComponent, {
    onSuccess: data => {
      if (!version?.id) return
      if (data.component) {
        queryClient.setQueryData(
          createConnectQueryKey(listPriceComponents, { planVersionId: version.id }),
          createProtobufSafeUpdater(listPriceComponents, prev => ({
            components: [...(prev?.components ?? []), data.component!],
          }))
        )
      }
      navigate('..')
    },
  })

  // Handle adding an existing product
  const handleAddExistingProduct = ({
    productId,
    componentName,
    formData,
    feeType,
  }: {
    productId: string
    componentName: string
    formData: Record<string, unknown>
    feeType: ComponentFeeType
  }) => {
    if (!version?.id) return

    const pricingType = toPricingTypeFromFeeType(
      feeType,
      feeType === 'usage' ? (formData.usageModel as string) : undefined
    )
    const priceInputs = buildPriceInputs(pricingType, formData, currency)
    const product = buildExistingProductRef(productId)

    createComponent.mutate({
      planVersionId: version.id,
      name: componentName,
      product,
      prices: wrapAsNewPriceEntries(priceInputs),
    })
  }

  // Handle creating a new product + component (from custom flow)
  const handleCreateNewProduct = (formData: Record<string, unknown>) => {
    if (!version?.id || !selectedFeeType) return

    const pricingType = toPricingTypeFromFeeType(
      selectedFeeType,
      selectedFeeType === 'usage' ? (formData.usageModel as string) : undefined
    )
    const priceInputs = buildPriceInputs(pricingType, formData, currency)
    const product = buildNewProductRef(customName, selectedFeeType, formData)

    createComponent.mutate({
      planVersionId: version.id,
      name: customName,
      product,
      prices: wrapAsNewPriceEntries(priceInputs),
    })
  }

  const resetCustomFlow = () => {
    setCustomStep('identity')
    setCustomName('')
    setCustomDescription('')
    setSelectedFeeType(null)
    identityMethods.reset()
  }

  return (
    <Sheet open={true} onOpenChange={() => navigate('..')}>
      <SheetContent size="large">
        <SheetHeader className="border-b border-border pb-3 mb-3">
          <SheetTitle>Component Catalog</SheetTitle>
          <SheetDescription>Add a price component to your plan</SheetDescription>
        </SheetHeader>
        <Tabs defaultValue="library" className="flex flex-col h-[calc(100%-80px)]">
          <TabsList className="w-full grid grid-cols-2 mb-4">
            <TabsTrigger value="library">Library Products</TabsTrigger>
            <TabsTrigger value="custom" onClick={resetCustomFlow}>
              Custom Component
            </TabsTrigger>
          </TabsList>
          <TabsContent value="library" className="flex-1 overflow-hidden mt-0">
            <ScrollArea className="h-full">
              <ProductBrowser currency={currency} onAdd={handleAddExistingProduct} />
            </ScrollArea>
          </TabsContent>
          <TabsContent value="custom" className="flex-1 overflow-hidden mt-0">
            <ScrollArea className="h-full">
              <CustomCreationFlow
                step={customStep}
                name={customName}
                description={customDescription}
                selectedFeeType={selectedFeeType}
                identityMethods={identityMethods}
                currency={currency}
                onIdentitySubmit={data => {
                  setCustomName(data.productName)
                  setCustomDescription(data.description ?? '')
                  setCustomStep('feeType')
                }}
                onFeeTypeSelect={ft => {
                  setSelectedFeeType(ft)
                  setCustomStep('form')
                }}
                onBack={step => setCustomStep(step)}
                onSubmit={handleCreateNewProduct}
                submitLabel="Add to Plan"
              />
            </ScrollArea>
          </TabsContent>
        </Tabs>
      </SheetContent>
    </Sheet>
  )
}
