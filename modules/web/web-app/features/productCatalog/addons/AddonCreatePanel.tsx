import {
  useMutation,
} from '@connectrpc/connect-query'
import {
  Input,
  Label,
  RadioGroup,
  RadioGroupItem,
  ScrollArea,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Switch,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { InfoIcon } from 'lucide-react'
import { useId, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { CustomCreationFlow, IdentitySchema } from '@/features/addons/CustomCreationFlow'
import { ProductBrowser } from '@/features/plans/pricecomponents/ProductBrowser'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing/conversions'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  createAddOn,
  listAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import type { ComponentFeeType } from '@/features/pricing/conversions'

type InstanceMode = 'single' | 'multiple' | 'unlimited'

export const AddonCreatePanel = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const switchId = useId()

  const activeCurrenciesQuery = useQuery(listTenantCurrencies)
  const activeCurrencies = activeCurrenciesQuery.data?.currencies ?? []
  const [currency, setCurrency] = useState<string | undefined>(undefined)
  const [selfServiceable, setSelfServiceable] = useState(false)
  const [instanceMode, setInstanceMode] = useState<InstanceMode>('single')
  const [multipleMax, setMultipleMax] = useState(2)

  const [customStep, setCustomStep] = useState<'identity' | 'feeType' | 'form'>('identity')
  const [customName, setCustomName] = useState('')
  const [customDescription, setCustomDescription] = useState('')
  const [selectedFeeType, setSelectedFeeType] = useState<ComponentFeeType | null>(null)

  const identityMethods = useZodForm({
    schema: IdentitySchema,
    defaultValues: { productName: '', description: '' },
  })

  const maxInstancesPerSubscription =
    instanceMode === 'single' ? 1 : instanceMode === 'multiple' ? multipleMax : undefined

  const createAddOnMutation = useMutation(createAddOn, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: [listAddOns.service.typeName],
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
    if (!currency) return
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
      selfServiceable,
      maxInstancesPerSubscription,
    })
  }

  const handleCreateNewProduct = (formData: Record<string, unknown>) => {
    if (!selectedFeeType || !currency) return

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
      selfServiceable,
      maxInstancesPerSubscription,
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
        <div className="space-y-4 mb-4 pb-4 border-b border-border">
          <div className="flex items-center gap-3">
            <Label className="text-sm font-medium text-muted-foreground w-36">Currency</Label>
            <Select value={currency} onValueChange={setCurrency}>
              <SelectTrigger className="w-[120px]">
                <SelectValue placeholder="Select" />
              </SelectTrigger>
              <SelectContent>
                {activeCurrencies.map(c => (
                  <SelectItem key={c} value={c}>
                    {c}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center gap-3">
            <Label htmlFor={switchId} className="text-sm font-medium text-muted-foreground w-36">
              Self-service
            </Label>
            <Switch id={switchId} checked={selfServiceable} onCheckedChange={setSelfServiceable} />
          </div>
          <div className="flex items-start gap-3">
            <div className="flex items-center gap-1.5 w-36 pt-2 shrink-0">
              <Label className="text-sm font-medium text-muted-foreground">Instances</Label>
              <TooltipProvider delayDuration={100}>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <InfoIcon className="h-3.5 w-3.5 text-muted-foreground cursor-help" />
                  </TooltipTrigger>
                  <TooltipContent className="max-w-64">
                    How many times this add-on can be added to a single subscription.
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            </div>
            <RadioGroup
              value={instanceMode}
              onValueChange={v => setInstanceMode(v as InstanceMode)}
              className="flex items-center gap-4"
            >
              <div className="flex items-center gap-1.5">
                <RadioGroupItem value="single" id="inst-single" />
                <Label htmlFor="inst-single" className="text-sm font-normal cursor-pointer">
                  Single
                </Label>
              </div>
              <div className="flex items-center gap-1.5">
                <RadioGroupItem value="multiple" id="inst-multiple" />
                <Label htmlFor="inst-multiple" className="text-sm font-normal cursor-pointer">
                  Multiple
                </Label>
                {instanceMode === 'multiple' && (
                  <Input
                    type="number"
                    min={2}
                    value={multipleMax}
                    onChange={e => setMultipleMax(Math.max(2, parseInt(e.target.value) || 2))}
                    className="w-16 h-7 text-sm"
                  />
                )}
              </div>
              <div className="flex items-center gap-1.5">
                <RadioGroupItem value="unlimited" id="inst-unlimited" />
                <Label htmlFor="inst-unlimited" className="text-sm font-normal cursor-pointer">
                  Unlimited
                </Label>
              </div>
            </RadioGroup>
          </div>
        </div>
        {currency && (
          <Tabs defaultValue="library" className="flex flex-col h-[calc(100%-220px)]">
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
        )}
      </SheetContent>
    </Sheet>
  )
}

