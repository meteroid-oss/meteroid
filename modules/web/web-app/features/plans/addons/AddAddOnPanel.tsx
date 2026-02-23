import {
  createConnectQueryKey,
  disableQuery,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Badge,
  Button,
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
import { Check, ChevronDownIcon, ChevronRightIcon, Plus } from 'lucide-react'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { CustomCreationFlow, IdentitySchema } from '@/features/addons/CustomCreationFlow'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { PricingDetailsView } from '@/features/plans/pricecomponents/components/PricingDetailsView'
import { ProductBrowser } from '@/features/plans/pricecomponents/ProductBrowser'
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
    : disableQuery
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
    onError: error => {
      toast.error(error instanceof Error ? error.message : 'Failed to attach add-on')
    },
  })

  const createAddOnMutation = useMutation(createAddOn, {
    onSuccess: async data => {
      if (!version?.id || !data.addOn) return
      try {
        await attachMutation.mutateAsync({
          planVersionId: version.id,
          addOnId: data.addOn.id,
        })
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : 'Failed to attach add-on to plan'
        )
      }
    },
    onError: error => {
      toast.error(error instanceof Error ? error.message : 'Failed to create add-on')
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
          <TabsList className="w-full grid grid-cols-3 mb-4">
            <TabsTrigger value="catalog">Catalog</TabsTrigger>
            <TabsTrigger value="product">From Product</TabsTrigger>
            <TabsTrigger value="custom" onClick={resetCustomFlow}>
              Custom
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
          <TabsContent value="product" className="flex-1 overflow-hidden mt-0">
            <ScrollArea className="h-full">
              <ProductBrowser
                currency={currency}
                onAdd={handleAddExistingProduct}
                submitLabel="Create & Attach"
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
                submitLabel="Create & Attach"
              />
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
  const [expandedId, setExpandedId] = useState<string | null>(null)

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
            const isExpanded = expandedId === addOn.id
            const componentFeeType = feeTypeEnumToComponentFeeType(addOn.feeType)
            const Icon = feeTypeIcon(componentFeeType)
            const feeLabel = feeTypeToHuman(componentFeeType)
            const priceBadges = priceSummaryBadges(componentFeeType, addOn.price, currency)
            const cadence = addOn.price ? formatCadence(addOn.price.cadence) : '-'

            return (
              <div
                key={addOn.id}
                className="border border-border rounded-lg bg-card"
              >
                <div className="flex items-center gap-3 p-3">
                  <button
                    type="button"
                    className="shrink-0 text-muted-foreground hover:text-foreground"
                    onClick={() => setExpandedId(isExpanded ? null : addOn.id)}
                  >
                    {isExpanded ? (
                      <ChevronDownIcon className="w-4 h-4" />
                    ) : (
                      <ChevronRightIcon className="w-4 h-4" />
                    )}
                  </button>
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
                {isExpanded && addOn.price && (
                  <div className="px-3 pb-3 pt-0 border-t border-border mt-0">
                    <PricingDetailsView prices={[addOn.price]} currency={currency} />
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

