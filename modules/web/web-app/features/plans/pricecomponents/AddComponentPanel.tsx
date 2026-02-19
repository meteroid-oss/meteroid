import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Button,
  Form,
  InputFormField,
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
import { ArrowLeftIcon, PencilIcon } from 'lucide-react'
import { createElement, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { feeTypeIcon, feeTypeToHuman, useCurrency } from '@/features/plans/pricecomponents/utils'
import type { ComponentFeeType } from '@/features/pricing/conversions'
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

import { FeeTypePicker } from './FeeTypePicker'
import { ProductBrowser } from './ProductBrowser'
import { ProductPricingForm } from './ProductPricingForm'

// --- Identity schema ---

const IdentitySchema = z.object({
  productName: z.string().min(1, 'Product name is required'),
  description: z.string().max(2048).optional(),
})

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
    console.log('Adding existing product with data:', {
      productId,
      componentName,
      formData,
      feeType,
    })

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
              />
            </ScrollArea>
          </TabsContent>
        </Tabs>
      </SheetContent>
    </Sheet>
  )
}

// --- Custom creation stepped flow ---

interface CustomCreationFlowProps {
  step: 'identity' | 'feeType' | 'form'
  name: string
  description: string
  selectedFeeType: ComponentFeeType | null
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  identityMethods: any
  currency: string
  onIdentitySubmit: (data: { productName: string; description?: string }) => void
  onFeeTypeSelect: (feeType: ComponentFeeType) => void
  onBack: (step: 'identity' | 'feeType') => void
  onSubmit: (formData: Record<string, unknown>) => void
}

const CustomCreationFlow = ({
  step,
  name,
  description,
  selectedFeeType,
  identityMethods,
  currency,
  onIdentitySubmit,
  onFeeTypeSelect,
  onBack,
  onSubmit,
}: CustomCreationFlowProps) => {
  switch (step) {
    case 'identity':
      return (
        <Form {...identityMethods}>
          <div className="space-y-4">
            <InputFormField
              name="productName"
              label="Product name"
              control={identityMethods.control}
            />
            <InputFormField
              name="description"
              label="Description (optional)"
              control={identityMethods.control}
            />
            <div className="flex justify-end pt-2">
              <Button type="button" onClick={identityMethods.handleSubmit(onIdentitySubmit)}>
                Next
              </Button>
            </div>
          </div>
        </Form>
      )
    case 'feeType':
      return (
        <div className="space-y-4">
          <button
            type="button"
            onClick={() => onBack('identity')}
            className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            <ArrowLeftIcon size={14} />
            Back
          </button>
          <FeeTypePicker onSelect={onFeeTypeSelect} />
        </div>
      )
    case 'form':
      if (!selectedFeeType) return null
      return (
        <div className="space-y-4">
          <button
            type="button"
            onClick={() => onBack('feeType')}
            className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            <ArrowLeftIcon size={14} />
            Back
          </button>
          {/* Card header */}
          <div className="rounded-lg border border-border bg-card">
            <div className="flex items-center gap-3 px-4 py-3 border-b border-border">
              <span className="text-muted-foreground">
                {createElement(feeTypeIcon(selectedFeeType), { size: 20 })}
              </span>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium truncate">{name}</span>
                  <button
                    type="button"
                    onClick={() => onBack('identity')}
                    className="text-muted-foreground hover:text-foreground transition-colors"
                  >
                    <PencilIcon size={12} />
                  </button>
                </div>
                {description && (
                  <span className="text-xs text-muted-foreground truncate block">
                    {description}
                  </span>
                )}
              </div>
              <span className="text-xs text-muted-foreground">
                {feeTypeToHuman(selectedFeeType)}
              </span>
            </div>
            <div className="p-4">
              <ProductPricingForm
                feeType={selectedFeeType}
                currency={currency}
                editableStructure
                onSubmit={onSubmit}
                submitLabel="Add to Plan"
              />
            </div>
          </div>
        </div>
      )
  }
}
