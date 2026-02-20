import {
  createConnectQueryKey,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Badge,
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
import { ArrowLeftIcon, Check, PencilIcon, Plus } from 'lucide-react'
import { createElement, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { FeeTypePicker } from '@/features/plans/pricecomponents/FeeTypePicker'
import { ProductBrowser } from '@/features/plans/pricecomponents/ProductBrowser'
import { ProductPricingForm } from '@/features/plans/pricecomponents/ProductPricingForm'
import {
  feeTypeIcon,
  feeTypeToHuman,
  priceSummaryBadges,
  useCurrency,
} from '@/features/plans/pricecomponents/utils'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing/conversions'
import { formatCadence } from '@/lib/mapping/prices'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  attachAddOnToPlanVersion,
  createAddOn,
  listAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { ComponentFeeType } from '@/features/pricing/conversions'

const IdentitySchema = z.object({
  productName: z.string().min(1, 'Product name is required'),
  description: z.string().max(2048).optional(),
})

export const AddAddOnPanel = () => {
  const navigate = useNavigate()
  const { version } = usePlanWithVersion()
  const queryClient = useQueryClient()
  const currency = useCurrency()

  const [customStep, setCustomStep] = useState<'identity' | 'feeType' | 'form'>('identity')
  const [customName, setCustomName] = useState('')
  const [customDescription, setCustomDescription] = useState('')
  const [selectedFeeType, setSelectedFeeType] = useState<ComponentFeeType | null>(null)
  const [catalogSearch, setCatalogSearch] = useState('')

  const identityMethods = useZodForm({
    schema: IdentitySchema,
    defaultValues: { productName: '', description: '' },
  })

  // Query all catalog add-ons (no plan filter)
  const catalogAddOns = useQuery(listAddOns, {
    pagination: { perPage: 100, page: 0 },
    search: catalogSearch || undefined,
  })

  // Query add-ons already attached to this plan version
  const planAddOns = useQuery(listAddOns, version?.id
    ? { planVersionId: version.id, pagination: { perPage: 100, page: 0 } }
    : { pagination: { perPage: 100, page: 0 } }
  )

  const attachedIds = new Set(planAddOns.data?.addOns?.map(a => a.id) ?? [])

  const attachMutation = useMutation(attachAddOnToPlanVersion, {
    onSuccess: () => {
      if (version?.id) {
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey(listAddOns, { planVersionId: version.id }),
        })
      }
      navigate('..')
    },
  })

  const createAddOnMutation = useMutation(createAddOn, {
    onSuccess: async data => {
      if (!version?.id || !data.addOn) return
      // After creating the catalog add-on, attach it to this plan version
      await attachMutation.mutateAsync({
        planVersionId: version.id,
        addOnId: data.addOn.id,
      })
    },
  })

  const handleAttachExisting = (addOn: AddOn) => {
    if (!version?.id) return
    attachMutation.mutate({
      planVersionId: version.id,
      addOnId: addOn.id,
    })
  }

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

    createAddOnMutation.mutate({
      name: componentName,
      product,
      price: wrapAsNewPriceEntries(priceInputs)[0],
    })
  }

  const handleCreateNewProduct = (formData: Record<string, unknown>) => {
    if (!version?.id || !selectedFeeType) return

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
          <SheetTitle>Add-on Catalog</SheetTitle>
          <SheetDescription>Attach an existing add-on or create a new one</SheetDescription>
        </SheetHeader>
        <Tabs defaultValue="catalog" className="flex flex-col h-[calc(100%-80px)]">
          <TabsList className="w-full grid grid-cols-2 mb-4">
            <TabsTrigger value="catalog">From Catalog</TabsTrigger>
            <TabsTrigger value="create" onClick={resetCustomFlow}>
              Create New
            </TabsTrigger>
          </TabsList>
          <TabsContent value="catalog" className="flex-1 overflow-hidden mt-0">
            <ScrollArea className="h-full">
              <CatalogBrowser
                addOns={catalogAddOns.data?.addOns ?? []}
                attachedIds={attachedIds}
                currency={currency}
                search={catalogSearch}
                onSearchChange={setCatalogSearch}
                onAttach={handleAttachExisting}
                isAttaching={attachMutation.isPending}
              />
            </ScrollArea>
          </TabsContent>
          <TabsContent value="create" className="flex-1 overflow-hidden mt-0">
            <ScrollArea className="h-full">
              <Tabs defaultValue="library" className="space-y-4">
                <TabsList className="w-full grid grid-cols-2">
                  <TabsTrigger value="library">From Product</TabsTrigger>
                  <TabsTrigger value="custom" onClick={resetCustomFlow}>
                    Custom
                  </TabsTrigger>
                </TabsList>
                <TabsContent value="library" className="mt-0">
                  <ProductBrowser
                    currency={currency}
                    onAdd={handleAddExistingProduct}
                    submitLabel="Create & Attach"
                  />
                </TabsContent>
                <TabsContent value="custom" className="mt-0">
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
                </TabsContent>
              </Tabs>
            </ScrollArea>
          </TabsContent>
        </Tabs>
      </SheetContent>
    </Sheet>
  )
}

// --- Catalog browser ---

interface CatalogBrowserProps {
  addOns: AddOn[]
  attachedIds: Set<string>
  currency: string
  search: string
  onSearchChange: (s: string) => void
  onAttach: (addOn: AddOn) => void
  isAttaching: boolean
}

const CatalogBrowser = ({
  addOns,
  attachedIds,
  currency,
  search,
  onSearchChange,
  onAttach,
  isAttaching,
}: CatalogBrowserProps) => {
  return (
    <div className="space-y-4">
      <div className="relative">
        <input
          type="search"
          placeholder="Search add-ons..."
          value={search}
          onChange={e => onSearchChange(e.target.value)}
          className="w-full h-9 pl-3 pr-3 border border-border rounded-md text-sm bg-background"
        />
      </div>
      {addOns.length === 0 ? (
        <p className="text-sm text-muted-foreground text-center py-8">
          No add-ons found. Create one from the &quot;Create New&quot; tab.
        </p>
      ) : (
        <div className="space-y-2">
          {addOns.map(addOn => {
            const isAttached = attachedIds.has(addOn.id)
            const componentFeeType = feeTypeEnumToComponentFeeType(addOn.feeType)
            const Icon = feeTypeIcon(componentFeeType)
            const feeLabel = feeTypeToHuman(componentFeeType)
            const priceBadges = priceSummaryBadges(componentFeeType, addOn.price, currency)
            const cadence = addOn.price ? formatCadence(addOn.price.cadence) : '-'

            return (
              <div
                key={addOn.id}
                className="flex items-center gap-3 p-3 border border-border rounded-lg bg-card"
              >
                <Icon className="w-4 h-4 text-muted-foreground shrink-0" />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium truncate">{addOn.name}</span>
                    <Badge variant="outline" size="sm">
                      {feeLabel}
                    </Badge>
                  </div>
                  <div className="flex items-center gap-2 mt-0.5">
                    <span className="text-xs text-muted-foreground">
                      {priceBadges.join(' / ')}
                    </span>
                    <span className="text-xs text-muted-foreground">{cadence}</span>
                  </div>
                </div>
                {isAttached ? (
                  <Badge variant="secondary" size="sm" className="shrink-0">
                    <Check className="h-3 w-3 mr-1" />
                    Attached
                  </Badge>
                ) : (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => onAttach(addOn)}
                    disabled={isAttaching}
                    className="shrink-0"
                  >
                    <Plus className="h-3 w-3 mr-1" />
                    Attach
                  </Button>
                )}
              </div>
            )
          })}
        </div>
      )}
    </div>
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
                submitLabel="Create & Attach"
              />
            </div>
          </div>
        </div>
      )
  }
}
