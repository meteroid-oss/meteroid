import { disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  Input,
  Label,
  RadioGroup,
  RadioGroupItem,
  ScrollArea,
  Separator,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Skeleton,
  Switch,
  Textarea,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { InfoIcon } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { toast } from 'sonner'

import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { extractStructuralInfo } from '@/features/plans/pricecomponents/ProductBrowser'
import { ProductPricingForm } from '@/features/plans/pricecomponents/ProductPricingForm'
import {
  buildPriceInputs,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing/conversions'
import { useQuery } from '@/lib/connectrpc'
import {
  editAddOn,
  getAddOn,
  listAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

import type { ComponentFeeType } from '@/features/pricing/conversions'

type InstanceMode = 'single' | 'multiple' | 'unlimited'

function deriveInstanceMode(max?: number): InstanceMode {
  if (max === undefined || max === null) return 'unlimited'
  if (max === 1) return 'single'
  return 'multiple'
}

export const AddonEditPanel = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const switchId = useId()
  const { addonId } = useParams<{ addonId: string }>()

  const addonQuery = useQuery(getAddOn, addonId ? { addOnId: addonId } : disableQuery)
  const addOn = addonQuery.data?.addOn
  const isLoading = addonQuery.isLoading

  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [selfServiceable, setSelfServiceable] = useState(false)
  const [instanceMode, setInstanceMode] = useState<InstanceMode>('single')
  const [multipleMax, setMultipleMax] = useState(2)
  const [initialized, setInitialized] = useState(false)

  useEffect(() => {
    if (addOn && !initialized) {
      setName(addOn.name)
      setDescription(addOn.description ?? '')
      setSelfServiceable(addOn.selfServiceable)
      const mode = deriveInstanceMode(addOn.maxInstancesPerSubscription)
      setInstanceMode(mode)
      if (mode === 'multiple' && addOn.maxInstancesPerSubscription) {
        setMultipleMax(addOn.maxInstancesPerSubscription)
      }
      setInitialized(true)
    }
  }, [addOn, initialized])

  const maxInstancesPerSubscription =
    instanceMode === 'single' ? 1 : instanceMode === 'multiple' ? multipleMax : undefined

  const editAddOnMutation = useMutation(editAddOn, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: [listAddOns.service.typeName],
      })
      queryClient.invalidateQueries({
        queryKey: [getAddOn.service.typeName],
      })
      navigate('..')
    },
    onError: error => {
      toast.error(`Failed to update add-on: ${error.message}`)
    },
  })

  const componentFeeType: ComponentFeeType | undefined = addOn
    ? feeTypeEnumToComponentFeeType(addOn.feeType)
    : undefined

  const structural = addOn?.feeStructure
    ? extractStructuralInfo(componentFeeType, addOn.feeStructure)
    : {}

  const currency = addOn?.price?.currency

  const handleSubmit = (formData: Record<string, unknown>) => {
    if (!addOn || !componentFeeType || !currency) return

    const pricingType = toPricingTypeFromFeeType(
      componentFeeType,
      componentFeeType === 'usage' ? (formData.usageModel as string) : undefined
    )
    const priceInputs = buildPriceInputs(pricingType, formData, currency)

    editAddOnMutation.mutate({
      addOnId: addOn.id,
      name,
      price: wrapAsNewPriceEntries(priceInputs)[0],
      description: description || undefined,
      selfServiceable,
      maxInstancesPerSubscription,
    })
  }

  return (
    <Sheet open={true} onOpenChange={() => navigate('..')}>
      <SheetContent size="large">
        <SheetHeader className="border-b border-border pb-3 mb-3">
          <SheetTitle>Edit Add-on</SheetTitle>
          <SheetDescription>Update pricing and settings</SheetDescription>
        </SheetHeader>

        {isLoading && (
          <div className="flex flex-col gap-4 py-4">
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-64" />
            <Skeleton className="h-32 w-full" />
          </div>
        )}

        {addOn && !isLoading && initialized && (
          <ScrollArea className="h-[calc(100%-80px)]">
            <div className="space-y-4 mb-4 pb-4 border-b border-border">
              <div className="flex items-center gap-3">
                <Label className="text-sm font-medium text-muted-foreground w-36">Name</Label>
                <Input
                  value={name}
                  onChange={e => setName(e.target.value)}
                  className="flex-1"
                />
              </div>
              <div className="flex items-start gap-3">
                <Label className="text-sm font-medium text-muted-foreground w-36 pt-2">
                  Description
                </Label>
                <Textarea
                  value={description}
                  onChange={e => setDescription(e.target.value)}
                  className="flex-1"
                  rows={2}
                />
              </div>
              <div className="flex items-center gap-3">
                <Label
                  htmlFor={switchId}
                  className="text-sm font-medium text-muted-foreground w-36"
                >
                  Self-service
                </Label>
                <Switch
                  id={switchId}
                  checked={selfServiceable}
                  onCheckedChange={setSelfServiceable}
                />
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

            <Separator className="my-4" />

            {componentFeeType && currency && (
              <div className="pb-4">
                <h3 className="text-sm font-medium text-muted-foreground mb-4">Pricing</h3>
                <ProductPricingForm
                  feeType={componentFeeType}
                  currency={currency}
                  existingPrice={addOn.price}
                  structuralInfo={structural}
                  onSubmit={handleSubmit}
                  submitLabel={
                    editAddOnMutation.isPending ? 'Saving...' : 'Save Changes'
                  }
                />
              </div>
            )}
          </ScrollArea>
        )}
      </SheetContent>
    </Sheet>
  )
}
