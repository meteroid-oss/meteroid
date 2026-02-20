import {
  createConnectQueryKey,
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

import { FeeTypePicker } from '@/features/plans/pricecomponents/FeeTypePicker'
import { ProductBrowser } from '@/features/plans/pricecomponents/ProductBrowser'
import { ProductPricingForm } from '@/features/plans/pricecomponents/ProductPricingForm'
import { feeTypeIcon, feeTypeToHuman } from '@/features/plans/pricecomponents/utils'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing/conversions'
import { useZodForm } from '@/hooks/useZodForm'
import {
  createAddOn,
  listAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

import type { ComponentFeeType } from '@/features/pricing/conversions'

const IdentitySchema = z.object({
  productName: z.string().min(1, 'Product name is required'),
  description: z.string().max(2048).optional(),
})

export const AddonCreatePanel = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  // Use a default currency for the catalog - TODO: get from tenant settings
  const currency = 'USD'

  const [customStep, setCustomStep] = useState<'identity' | 'feeType' | 'form'>('identity')
  const [customName, setCustomName] = useState('')
  const [customDescription, setCustomDescription] = useState('')
  const [selectedFeeType, setSelectedFeeType] = useState<ComponentFeeType | null>(null)

  const identityMethods = useZodForm({
    schema: IdentitySchema,
    defaultValues: { productName: '', description: '' },
  })

  const createAddOnMutation = useMutation(createAddOn, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listAddOns, {}),
      })
      navigate('..')
    },
  })

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
    const pricingType = toPricingTypeFromFeeType(
      feeType,
      feeType === 'usage' ? (formData.usageModel as string) : undefined
    )
    const priceInputs = buildPriceInputs(pricingType, formData, currency)
    const product = buildExistingProductRef(productId)

    createAddOnMutation.mutate({
      name: componentName,
      product,
      price: wrapAsNewPriceEntries(priceInputs)[0],
    })
  }

  const handleCreateNewProduct = (formData: Record<string, unknown>) => {
    if (!selectedFeeType) return

    const pricingType = toPricingTypeFromFeeType(
      selectedFeeType,
      selectedFeeType === 'usage' ? (formData.usageModel as string) : undefined
    )
    const priceInputs = buildPriceInputs(pricingType, formData, currency)
    const product = buildNewProductRef(customName, selectedFeeType, formData)

    createAddOnMutation.mutate({
      name: customName,
      description: customDescription || undefined,
      product,
      price: wrapAsNewPriceEntries(priceInputs)[0],
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
          <SheetTitle>New Add-on</SheetTitle>
          <SheetDescription>Create a new catalog add-on</SheetDescription>
        </SheetHeader>
        <Tabs defaultValue="library" className="flex flex-col h-[calc(100%-80px)]">
          <TabsList className="w-full grid grid-cols-2 mb-4">
            <TabsTrigger value="library">From Product</TabsTrigger>
            <TabsTrigger value="custom" onClick={resetCustomFlow}>
              Custom
            </TabsTrigger>
          </TabsList>
          <TabsContent value="library" className="flex-1 overflow-hidden mt-0">
            <ScrollArea className="h-full">
              <ProductBrowser
                currency={currency}
                onAdd={handleAddExistingProduct}
                submitLabel="Create Add-on"
              />
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
                submitLabel="Create Add-on"
              />
            </div>
          </div>
        </div>
      )
  }
}
