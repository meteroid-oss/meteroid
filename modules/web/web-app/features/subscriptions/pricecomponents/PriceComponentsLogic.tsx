import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
  ScrollArea,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Textarea,
} from '@ui/components'
import { cn } from '@ui/lib'
import {
  Activity,
  Calendar,
  Check,
  Edit2,
  Package,
  Plus,
  PlusIcon,
  RefreshCcw,
  Settings,
  Users,
  X,
  Zap,
} from 'lucide-react'
import { useMemo, useEffect, useState } from 'react'

import { FeeTypePicker } from '@/features/plans/pricecomponents/FeeTypePicker'
import { ProductBrowser, extractStructuralInfo } from '@/features/plans/pricecomponents/ProductBrowser'
import { ProductPricingForm } from '@/features/plans/pricecomponents/ProductPricingForm'
import { type ComponentFeeType, feeTypeFromPrice, formDataToPrice } from '@/features/pricing'
import { useQuery } from '@/lib/connectrpc'
import {
  formatUsagePriceSummary,
  getBillingPeriodLabel,
  getPrice,
  getPriceBillingLabel,
  getPriceUnitPrice,
} from '@/lib/mapping/priceToSubscriptionFee'
import {
  ComponentOverride,
  ComponentParameterization,
  ExtraComponent,
} from '@/pages/tenants/subscription/create/state'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { PriceComponent as ProtoPriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { getProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

// --- State types ---

export interface PriceComponentsState {
  components: {
    removed: string[]
    parameterized: ComponentParameterization[]
    overridden: ComponentOverride[]
    extra: ExtraComponent[]
  }
}

// --- Helpers ---

function deriveFeeType(component: ProtoPriceComponent): ComponentFeeType {
  const price = component.prices[0]
  if (!price) return 'rate'
  return feeTypeFromPrice(price)
}

function feeTypeIcon(feeType: ComponentFeeType) {
  switch (feeType) {
    case 'rate':
      return <Calendar className="h-4 w-4" />
    case 'usage':
      return <Activity className="h-4 w-4" />
    case 'slot':
      return <Users className="h-4 w-4" />
    case 'capacity':
      return <Zap className="h-4 w-4" />
    default:
      return <Package className="h-4 w-4" />
  }
}

function feeTypeLabel(feeType: ComponentFeeType): string {
  switch (feeType) {
    case 'rate':
      return 'Fixed'
    case 'usage':
      return 'Usage'
    case 'slot':
      return 'Per Seat'
    case 'capacity':
      return 'Capacity'
    case 'oneTime':
      return 'One-time'
    case 'extraRecurring':
      return 'Recurring'
  }
}

function formatCurrency(price: string | number, currency: string): string {
  const amount = typeof price === 'string' ? parseFloat(price || '0') : price
  return amount.toLocaleString(undefined, {
    style: 'currency',
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })
}

function getComponentBillingLabel(
  component: ProtoPriceComponent,
  configuration?: ComponentParameterization
): string {
  if (configuration?.billingPeriod !== undefined) {
    return getBillingPeriodLabel(configuration.billingPeriod)
  }
  const price = component.prices[0]
  if (price) return getPriceBillingLabel(price)
  return 'Monthly'
}

function getDisplayUnitPrice(
  component: ProtoPriceComponent,
  configuration?: ComponentParameterization,
  override?: ComponentOverride,
  currency?: string
): number {
  // Override takes priority — derive display price from formData
  if (override && currency) {
    const displayPrice = formDataToPrice(override.feeType, override.formData, currency)
    return getPriceUnitPrice(displayPrice)
  }

  // For capacity: match selected threshold
  if (configuration?.committedCapacity !== undefined) {
    const matched = component.prices.find(
      p =>
        p.pricing.case === 'capacityPricing' &&
        p.pricing.value.included === configuration.committedCapacity
    )
    if (matched) return getPriceUnitPrice(matched)
  }

  const price = component.prices[0]
  return price ? getPriceUnitPrice(price) : 0
}

// --- Main component ---

interface PriceComponentsLogicProps {
  planVersionId: PlanVersion['id']
  currency: string
  state: PriceComponentsState
  onStateChange: (state: PriceComponentsState) => void
  onValidationChange?: (isValid: boolean, errors: string[]) => void
}

export const PriceComponentsLogic = ({
  planVersionId,
  currency,
  state,
  onStateChange,
  onValidationChange,
}: PriceComponentsLogicProps) => {
  const [editingComponentId, setEditingComponentId] = useState<string | null>(null)
  const [showAddFeeModal, setShowAddFeeModal] = useState(false)
  const [overrideComponentId, setOverrideComponentId] = useState<string | null>(null)
  const [editExtraIndex, setEditExtraIndex] = useState<number | null>(null)

  const componentsQuery = useQuery(
    listPriceComponents,
    { planVersionId: planVersionId ?? '' },
    { enabled: Boolean(planVersionId) }
  )
  const planComponents = componentsQuery?.data?.components ?? []

  const setState = (updater: (prev: PriceComponentsState) => PriceComponentsState) => {
    onStateChange(updater(state))
  }

  // --- Configuration logic ---

  const requiresConfiguration = (component: ProtoPriceComponent): boolean => {
    if (state.components.overridden.some(o => o.componentId === component.id)) return false
    const feeType = deriveFeeType(component)
    if (feeType === 'slot') return true
    if (feeType === 'capacity' && component.prices.length > 1) return true
    return false
  }

  const isComponentConfigured = (component: ProtoPriceComponent): boolean => {
    if (!requiresConfiguration(component)) return true
    if (state.components.overridden.some(o => o.componentId === component.id)) return true

    const config = state.components.parameterized.find(p => p.componentId === component.id)
    if (!config) return false

    const feeType = deriveFeeType(component)
    if (feeType === 'slot') return config.initialSlotCount !== undefined
    if (feeType === 'capacity' && component.prices.length > 1)
      return config.committedCapacity !== undefined
    return false
  }

  useEffect(() => {
    const unconfigured = planComponents.filter(c => {
      const isExcluded = state.components.removed.includes(c.id)
      return !isExcluded && requiresConfiguration(c) && !isComponentConfigured(c)
    })
    onValidationChange?.(
      unconfigured.length === 0,
      unconfigured.map(c => `${c.name} requires configuration`)
    )
  }, [state.components, planComponents, onValidationChange])

  // --- State mutations ---

  const toggleRemoval = (componentId: string) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        removed: prev.components.removed.includes(componentId)
          ? prev.components.removed.filter(id => id !== componentId)
          : [...prev.components.removed, componentId],
        parameterized: prev.components.removed.includes(componentId)
          ? prev.components.parameterized
          : prev.components.parameterized.filter(p => p.componentId !== componentId),
      },
    }))
  }

  const updateConfiguration = (componentId: string, config: Partial<ComponentParameterization>) => {
    setState(prev => {
      const existing = prev.components.parameterized.find(p => p.componentId === componentId)
      return {
        ...prev,
        components: {
          ...prev.components,
          parameterized: existing
            ? prev.components.parameterized.map(p =>
                p.componentId === componentId ? { ...p, ...config } : p
              )
            : [...prev.components.parameterized, { componentId, ...config }],
        },
      }
    })
  }

  const addOverride = (componentId: string, override: ComponentOverride) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        overridden: [
          ...prev.components.overridden.filter(o => o.componentId !== componentId),
          override,
        ],
        parameterized: prev.components.parameterized.filter(p => p.componentId !== componentId),
      },
    }))
  }

  const removeOverride = (componentId: string) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        overridden: prev.components.overridden.filter(o => o.componentId !== componentId),
      },
    }))
  }

  const addExtra = (component: ExtraComponent) => {
    setState(prev => ({
      ...prev,
      components: { ...prev.components, extra: [...prev.components.extra, component] },
    }))
  }

  const removeExtra = (index: number) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        extra: prev.components.extra.filter((_, i) => i !== index),
      },
    }))
  }

  const updateExtra = (index: number, component: ExtraComponent) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        extra: prev.components.extra.map((c, i) => (i === index ? component : c)),
      },
    }))
  }

  // --- Render ---

  return (
    <div className="space-y-3">
      {planComponents.map(component => {
        const isExcluded = state.components.removed.includes(component.id)
        const configuration = state.components.parameterized.find(
          p => p.componentId === component.id
        )
        const override = state.components.overridden.find(o => o.componentId === component.id)
        const isOverridden = !!override
        const isEditing = editingComponentId === component.id
        const configured = isComponentConfigured(component)
        const needsConfig =
          requiresConfiguration(component) && !configured && !isOverridden && !isExcluded

        return (
          <CompactPriceComponentCard
            key={component.id}
            component={component}
            isExcluded={isExcluded}
            isConfigured={configured}
            isOverridden={isOverridden}
            isEditing={isEditing}
            needsConfiguration={needsConfig}
            configuration={configuration}
            override={override}
            currency={currency}
            onToggleExclude={() => toggleRemoval(component.id)}
            onStartEdit={() => setEditingComponentId(component.id)}
            onEndEdit={() => setEditingComponentId(null)}
            onUpdateConfiguration={config => updateConfiguration(component.id, config)}
            onStartOverride={() => setOverrideComponentId(component.id)}
            onRemoveOverride={() => removeOverride(component.id)}
          />
        )
      })}

      {state.components.extra.map((extra, index) => (
        <ExtraComponentCard
          key={`extra-${index}`}
          component={extra}
          currency={currency}
          onEdit={() => setEditExtraIndex(index)}
          onRemove={() => removeExtra(index)}
        />
      ))}

      <Button
        type="button"
        variant="outline"
        className="w-full border-dashed"
        onClick={() => setShowAddFeeModal(true)}
      >
        <Plus className="h-4 w-4 mr-2" />
        Add a fee
      </Button>

      {!planComponents.length && !state.components.extra.length && (
        <span className="text-muted-foreground">No price components</span>
      )}

      {showAddFeeModal && (
        <AddFeeModal
          onClose={() => setShowAddFeeModal(false)}
          onAdd={addExtra}
          currency={currency}
        />
      )}

      {overrideComponentId &&
        (() => {
          const original = planComponents.find(c => c.id === overrideComponentId)
          return (
            <OverrideFeeModal
              componentId={overrideComponentId}
              originalComponent={original}
              onClose={() => setOverrideComponentId(null)}
              onSave={override => {
                addOverride(overrideComponentId, override)
                setOverrideComponentId(null)
              }}
              currency={currency}
            />
          )
        })()}

      {editExtraIndex !== null && (
        <AddFeeModal
          onClose={() => setEditExtraIndex(null)}
          onAdd={component => {
            updateExtra(editExtraIndex, component)
            setEditExtraIndex(null)
          }}
          currency={currency}
          initialValues={state.components.extra[editExtraIndex]}
          isEditing
        />
      )}
    </div>
  )
}

// --- Compact plan component card ---

interface CompactPriceComponentCardProps {
  component: ProtoPriceComponent
  isExcluded: boolean
  isConfigured: boolean
  isOverridden: boolean
  isEditing: boolean
  needsConfiguration: boolean
  configuration?: ComponentParameterization
  override?: ComponentOverride
  currency: string
  onToggleExclude: () => void
  onStartEdit: () => void
  onEndEdit: () => void
  onUpdateConfiguration: (config: Partial<ComponentParameterization>) => void
  onStartOverride: () => void
  onRemoveOverride: () => void
}

const CompactPriceComponentCard = ({
  component,
  isExcluded,
  isConfigured,
  isOverridden,
  isEditing,
  needsConfiguration,
  configuration,
  override,
  currency,
  onToggleExclude,
  onStartEdit,
  onEndEdit,
  onUpdateConfiguration,
  onStartOverride,
  onRemoveOverride,
}: CompactPriceComponentCardProps) => {
  const feeType = deriveFeeType(component)
  const unitPrice = getDisplayUnitPrice(component, configuration, override, currency)
  const billingLabel = getComponentBillingLabel(component, configuration)

  const canConfigure = () => {
    if (isOverridden) return false
    if (feeType === 'slot') return true
    if (feeType === 'capacity' && component.prices.length > 1) return true
    return false
  }

  const renderPriceDisplay = () => {
    if (needsConfiguration && !isExcluded) {
      return (
        <div className="text-right">
          <div className="text-xs font-medium text-red-700">Configuration required</div>
        </div>
      )
    }

    // Slot with configured count
    if (
      feeType === 'slot' &&
      configuration?.initialSlotCount &&
      configuration.initialSlotCount > 1
    ) {
      const total = unitPrice * configuration.initialSlotCount
      return (
        <div className="text-right">
          <div className="flex items-center justify-end gap-1">
            <span>
              {formatCurrency(unitPrice, currency)} × {configuration.initialSlotCount} slots
            </span>
            <Badge variant="secondary" size="sm">
              {billingLabel}
            </Badge>
          </div>
          <div className="text-xs text-muted-foreground">{formatCurrency(total, currency)}</div>
        </div>
      )
    }

    // Usage-based: show model + rate summary instead of €0.00
    if (feeType === 'usage') {
      const displayPrice = override
        ? formDataToPrice(override.feeType, override.formData, currency)
        : getPrice(component)
      const usage = displayPrice ? formatUsagePriceSummary(displayPrice, currency) : undefined
      return (
        <div className="text-right">
          <div className="flex items-center justify-end gap-1">
            {usage ? (
              <span>
                {usage.model && <><span className="text-muted-foreground">{usage.model}</span>{' '}</>}{usage.amount}
              </span>
            ) : (
              <span className="text-muted-foreground">Metered</span>
            )}
            <Badge variant="secondary" size="sm">
              {billingLabel}
            </Badge>
          </div>
        </div>
      )
    }

    return (
      <div className="text-right">
        <div className="flex items-center justify-end gap-1">
          <span>{formatCurrency(unitPrice, currency)}</span>
          <Badge variant="secondary" size="sm">
            {billingLabel}
          </Badge>
        </div>
      </div>
    )
  }

  const renderConfiguration = () => {
    if (!isEditing || isExcluded) return null

    if (feeType === 'slot') {
      return (
        <div className="space-y-2">
          <Label className="text-xs">Initial Seats</Label>
          <Input
            type="number"
            min="0"
            className="h-8 text-xs"
            placeholder="Number of seats"
            value={configuration?.initialSlotCount || ''}
            onChange={e => {
              const value = e.target.value ? parseInt(e.target.value) : undefined
              onUpdateConfiguration({ initialSlotCount: value })
            }}
          />
        </div>
      )
    }

    if (feeType === 'capacity' && component.prices.length > 1) {
      return (
        <div className="space-y-2">
          <Select
            value={configuration?.committedCapacity?.toString() || ''}
            onValueChange={value => onUpdateConfiguration({ committedCapacity: BigInt(value) })}
          >
            <SelectTrigger className="h-8 text-xs">
              <SelectValue placeholder="Select capacity" />
            </SelectTrigger>
            <SelectContent>
              {component.prices
                .filter(p => p.pricing.case === 'capacityPricing')
                .map(p => {
                  const cap = p.pricing.value as { included: bigint; rate: string }
                  return (
                    <SelectItem key={cap.included.toString()} value={cap.included.toString()}>
                      <div className="flex justify-between items-center w-full">
                        <span>{cap.included.toString()} included</span>
                        <span className="text-muted-foreground ml-2">
                          {formatCurrency(cap.rate, currency)}
                        </span>
                      </div>
                    </SelectItem>
                  )
                })}
            </SelectContent>
          </Select>
        </div>
      )
    }

    return null
  }

  return (
    <Card
      className={cn(
        'transition-all duration-200 hover:shadow-sm',
        isExcluded && 'opacity-50 bg-muted/30',
        needsConfiguration && 'bg-card text-card-foreground'
      )}
    >
      <CardHeader className="p-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3 flex-1">
            <div className="text-muted-foreground">{feeTypeIcon(feeType)}</div>
            <div className="flex-1">
              <div className="flex items-center gap-2">
                <CardTitle className={cn('text-sm font-medium', isExcluded && 'line-through')}>
                  {isOverridden && override?.name ? override.name : component.name}
                </CardTitle>
                <Badge variant="outline" size="sm">
                  {feeTypeLabel(feeType)}
                </Badge>
                {isOverridden && (
                  <Badge variant="destructive" size="sm">
                    Custom Price
                  </Badge>
                )}
              </div>
            </div>
            <div className={cn('text-sm font-medium', isExcluded && 'line-through')}>
              {renderPriceDisplay()}
            </div>
          </div>
          <div className="flex gap-1 ml-4">
            {!isOverridden && canConfigure() && (
              <Popover open={isEditing} onOpenChange={open => (open ? onStartEdit() : onEndEdit())}>
                <PopoverTrigger asChild>
                  <Button
                    variant={needsConfiguration && !isExcluded ? 'default' : 'ghost'}
                    size="sm"
                    disabled={isExcluded}
                    type="button"
                    className={needsConfiguration && !isExcluded ? 'animate-pulse' : ''}
                  >
                    {isEditing ? <Check className="h-3 w-3" /> : <Settings className="h-3 w-3" />}
                  </Button>
                </PopoverTrigger>
                <PopoverContent className="w-64" align="end">
                  <div className="space-y-2">
                    <h4 className="font-medium text-sm">Configure {component.name}</h4>
                    {renderConfiguration()}
                    <div className="flex justify-end gap-2 pt-2">
                      <Button size="sm" variant="ghost" onClick={onEndEdit} type="button">
                        Cancel
                      </Button>
                      <Button size="sm" onClick={onEndEdit} type="button">
                        Done
                      </Button>
                    </div>
                  </div>
                </PopoverContent>
              </Popover>
            )}
            {!isExcluded && (
              <>
                <Button
                  variant="ghost"
                  size="sm"
                  type="button"
                  onClick={onStartOverride}
                  title={isOverridden ? 'Edit custom pricing' : 'Override with custom pricing'}
                >
                  <Edit2 className="h-3 w-3" />
                </Button>
                {isOverridden && (
                  <Button
                    variant="ghost"
                    size="sm"
                    type="button"
                    onClick={onRemoveOverride}
                    title="Remove custom pricing"
                  >
                    <RefreshCcw className="h-3 w-3" />
                  </Button>
                )}
              </>
            )}
            <Button
              variant={isExcluded ? 'ghost' : 'destructive'}
              size="sm"
              type="button"
              onClick={onToggleExclude}
              title={isExcluded ? 'Include this component' : 'Exclude this component'}
            >
              {isExcluded ? <PlusIcon className="h-3 w-3" /> : <X className="h-3 w-3" />}
            </Button>
          </div>
        </div>
      </CardHeader>
      {isConfigured && !isEditing && (
        <CardContent className="px-3 pb-3 pt-0">
          <div className="text-xs text-muted-foreground">
            {configuration?.initialSlotCount !== undefined && (
              <span>Seats: {configuration.initialSlotCount}</span>
            )}
            {configuration?.committedCapacity !== undefined && (
              <span>
                {configuration.initialSlotCount !== undefined && ' • '}
                Capacity: {configuration.committedCapacity.toString()}
              </span>
            )}
          </div>
        </CardContent>
      )}
    </Card>
  )
}

// --- Extra component card ---

const ExtraComponentCard = ({
  component,
  currency,
  onEdit,
  onRemove,
}: {
  component: ExtraComponent
  currency: string
  onEdit: () => void
  onRemove: () => void
}) => {
  const displayPrice = formDataToPrice(component.feeType, component.formData, currency)
  const unitPrice = getPriceUnitPrice(displayPrice)
  const billingLabel = getPriceBillingLabel(displayPrice)
  const isUsage = displayPrice.pricing.case === 'usagePricing'

  return (
    <Card className="transition-all duration-200 hover:shadow-sm">
      <CardHeader className="p-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3 flex-1">
            <div className="text-muted-foreground">
              <Plus className="h-4 w-4" />
            </div>
            <div className="flex-1">
              <div className="flex items-center gap-2">
                <CardTitle className="text-sm font-medium">{component.name}</CardTitle>
                <Badge variant="secondary" size="sm">
                  Extra
                </Badge>
              </div>
            </div>
            <div className="text-sm font-medium text-right">
              <div className="flex items-center justify-end gap-1">
                {isUsage ? (
                  (() => {
                    const usage = formatUsagePriceSummary(displayPrice, currency)
                    return (
                      <span>
                        {usage.model && <><span className="text-muted-foreground">{usage.model}</span>{' '}</>}{usage.amount}
                      </span>
                    )
                  })()
                ) : (
                  <span>{formatCurrency(unitPrice, currency)}</span>
                )}
                <Badge variant="secondary" size="sm">
                  {billingLabel}
                </Badge>
              </div>
            </div>
          </div>
          <div className="flex gap-1 ml-4">
            <Button variant="ghost" size="sm" type="button" onClick={onEdit}>
              <Edit2 className="h-3 w-3" />
            </Button>
            <Button variant="destructive" size="sm" type="button" onClick={onRemove}>
              <X className="h-3 w-3" />
            </Button>
          </div>
        </div>
      </CardHeader>
    </Card>
  )
}

// --- Add Fee Modal — two tabs: product library + custom fee ---

interface AddFeeModalProps {
  onClose: () => void
  onAdd: (component: ExtraComponent) => void
  currency: string
  initialValues?: ExtraComponent
  isEditing?: boolean
}

const AddFeeModal = ({
  onClose,
  onAdd,
  currency,
  initialValues,
  isEditing = false,
}: AddFeeModalProps) => {
  // Custom tab state
  const [customStep, setCustomStep] = useState<'identity' | 'feeType' | 'pricing'>(
    initialValues ? 'pricing' : 'identity'
  )
  const [name, setName] = useState(initialValues?.name || '')
  const [description, setDescription] = useState(initialValues?.description || '')
  const [feeType, setFeeType] = useState<ComponentFeeType | null>(
    initialValues?.feeType ?? null
  )

  const handleFeeTypeSelect = (ft: ComponentFeeType) => {
    setFeeType(ft)
    setCustomStep('pricing')
  }

  const handleCustomSubmit = (formData: Record<string, unknown>) => {
    if (!feeType) return
    onAdd({ name, description: description || undefined, feeType, formData, productId: undefined })
    onClose()
  }

  const handleProductAdd = ({
    productId,
    componentName,
    formData,
    feeType: ft,
  }: {
    productId: string
    componentName: string
    formData: Record<string, unknown>
    feeType: ComponentFeeType
  }) => {
    onAdd({ name: componentName, feeType: ft, formData, productId })
    onClose()
  }

  // For editing, show just the pricing form directly
  if (isEditing) {
    return (
      <Dialog open={true} onOpenChange={onClose}>
        <DialogContent className="sm:max-w-[540px] max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Fee</DialogTitle>
          </DialogHeader>
          {feeType && (
            <div className="space-y-4">
              <div className="text-sm font-medium">
                {name} — {feeTypeLabel(feeType)}
              </div>
              <ProductPricingForm
                feeType={feeType}
                currency={currency}
                editableStructure
                onSubmit={handleCustomSubmit}
                submitLabel="Save Changes"
              />
            </div>
          )}
        </DialogContent>
      </Dialog>
    )
  }

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[600px] max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Add a Fee</DialogTitle>
        </DialogHeader>
        <Tabs defaultValue="product" className="flex flex-col flex-1 overflow-hidden">
          <TabsList className="w-full grid grid-cols-2">
            <TabsTrigger value="product">From Product</TabsTrigger>
            <TabsTrigger value="custom">Custom Fee</TabsTrigger>
          </TabsList>

          {/* Product Library Tab */}
          <TabsContent value="product" className="flex-1 overflow-hidden mt-4">
            <ScrollArea className="h-[55vh]">
              <ProductBrowser currency={currency} onAdd={handleProductAdd} submitLabel="Add Fee" />
            </ScrollArea>
          </TabsContent>

          {/* Custom Fee Tab */}
          <TabsContent value="custom" className="flex-1 overflow-y-auto mt-4">
            {customStep === 'identity' && (
              <div className="space-y-4">
                <div>
                  <Label className="text-sm">Product name</Label>
                  <Input
                    className="mt-1"
                    placeholder="e.g., Setup Fee, Custom Service"
                    value={name}
                    onChange={e => setName(e.target.value)}
                    autoFocus
                  />
                </div>
                <div>
                  <Label className="text-sm">Description (optional)</Label>
                  <Textarea
                    className="mt-1"
                    placeholder="Brief description"
                    value={description}
                    onChange={e => setDescription(e.target.value)}
                  />
                </div>
                {name.length > 0 && (
                  <div className="flex justify-end">
                    <Button type="button" onClick={() => setCustomStep('feeType')}>
                      Next
                    </Button>
                  </div>
                )}
              </div>
            )}

            {customStep === 'feeType' && (
              <div className="space-y-4">
                <button
                  type="button"
                  onClick={() => setCustomStep('identity')}
                  className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
                >
                  ← Back
                </button>
                <div className="text-sm font-medium">{name}</div>
                <FeeTypePicker onSelect={handleFeeTypeSelect} />
              </div>
            )}

            {customStep === 'pricing' && feeType && (
              <div className="space-y-4">
                <button
                  type="button"
                  onClick={() => setCustomStep('feeType')}
                  className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
                >
                  ← Back
                </button>
                <div className="text-sm font-medium">
                  {name} — {feeTypeLabel(feeType)}
                </div>
                <ProductPricingForm
                  feeType={feeType}
                  currency={currency}
                  editableStructure
                  onSubmit={handleCustomSubmit}
                  submitLabel="Add Fee"
                />
              </div>
            )}
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  )
}

// --- Override Fee Modal — reuses ProductPricingForm ---

interface OverrideFeeModalProps {
  componentId: string
  originalComponent?: ProtoPriceComponent
  onClose: () => void
  onSave: (override: ComponentOverride) => void
  currency: string
}

const OverrideFeeModal = ({
  componentId,
  originalComponent,
  onClose,
  onSave,
  currency,
}: OverrideFeeModalProps) => {
  const feeType = originalComponent ? deriveFeeType(originalComponent) : 'rate'
  const hasProduct = !!originalComponent?.productId

  const productQuery = useQuery(
    getProduct,
    originalComponent?.productId ? { productId: originalComponent.productId } : {},
    { enabled: hasProduct }
  )
  const product = productQuery.data?.product

  const structural = useMemo(
    () => (product ? extractStructuralInfo(feeType, product.feeStructure) : undefined),
    [feeType, product]
  )

  const handleSubmit = (formData: Record<string, unknown>) => {
    onSave({
      componentId,
      name: originalComponent?.name || '',
      feeType,
      formData,
      productId: originalComponent?.productId,
    })
  }

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[500px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Override: {originalComponent?.name}</DialogTitle>
        </DialogHeader>
        <ProductPricingForm
          feeType={feeType}
          currency={currency}
          existingPrice={originalComponent ? getPrice(originalComponent) : undefined}
          structuralInfo={structural}
          editableStructure={!hasProduct}
          onSubmit={handleSubmit}
          submitLabel="Save Override"
        />
      </DialogContent>
    </Dialog>
  )
}

