import { zodResolver } from '@hookform/resolvers/zod'
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
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@ui/components'
import { useAtom } from 'jotai'
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
import React, { useState } from 'react'
import { useForm } from 'react-hook-form'
import { z } from 'zod'

import {
  getBillingPeriodLabel,
  getExtraComponentBillingPeriodLabel,
  getSchemaComponentBillingPeriodLabel,
  mapTermToBillingPeriod,
} from '@/features/subscriptions/utils/billingPeriods'
import { useQuery } from '@/lib/connectrpc'
import { mapFeeType } from '@/lib/mapping/feesFromGrpc'
import { PriceComponent } from '@/lib/schemas/plans'
import {
  ComponentOverride,
  ComponentParameterization,
  createSubscriptionAtom,
  ExtraComponent,
} from '@/pages/tenants/subscription/create/state'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { Fee_BillingType } from '@/rpc/api/pricecomponents/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { BillingPeriod as SharedBillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import {
  SubscriptionComponentNewInternal,
  SubscriptionFee,
  SubscriptionFee_CapacitySubscriptionFee,
  SubscriptionFee_ExtraRecurringSubscriptionFee,
  SubscriptionFee_OneTimeSubscriptionFee,
  SubscriptionFee_RateSubscriptionFee,
  SubscriptionFee_SlotSubscriptionFee,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'
import { cn } from '@ui/lib'

export const CreateSubscriptionPriceComponents = ({
  planVersionId,
  customerId,
  onValidationChange,
}: {
  planVersionId: PlanVersion['id']
  customerId?: string
  onValidationChange?: (isValid: boolean, errors: string[]) => void
}) => {
  const [state, setState] = useAtom(createSubscriptionAtom)
  const [editingComponentId, setEditingComponentId] = useState<string | null>(null)
  const [showAddFeeModal, setShowAddFeeModal] = useState(false)
  const [overrideComponentId, setOverrideComponentId] = useState<string | null>(null)
  const [editExtraIndex, setEditExtraIndex] = useState<number | null>(null)

  const planPriceComponentsQuery = useQuery(
    listPriceComponents,
    {
      planVersionId: planVersionId ?? '',
    },
    { enabled: Boolean(planVersionId) }
  )

  const customerQuery = useQuery(getCustomerById, { id: customerId! }, { enabled: !!customerId })

  const planPriceComponents = planPriceComponentsQuery?.data?.components.map(
    c =>
      ({
        id: c.id,
        name: c.name,
        localId: c.localId,
        fee: c.fee ? mapFeeType(c.fee.feeType) : undefined,
        productId: c.productId,
      }) as PriceComponent
  )

  const customer = customerQuery.data?.customer
  const currency = customer?.currency || 'USD'

  // Validation effect
  React.useEffect(() => {
    const unconfiguredComponents = getUnconfiguredComponents()
    const isValid = unconfiguredComponents.length === 0
    const errors = unconfiguredComponents.map(c => `${c.name} requires configuration`)

    onValidationChange?.(isValid, errors)
  }, [state.components, planPriceComponents, onValidationChange])

  const toggleComponentRemoval = (componentId: string) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        removed: prev.components.removed.includes(componentId)
          ? prev.components.removed.filter(id => id !== componentId)
          : [...prev.components.removed, componentId],
        // Clear any configuration if removing
        parameterized: prev.components.removed.includes(componentId)
          ? prev.components.parameterized
          : prev.components.parameterized.filter(p => p.componentId !== componentId),
      },
    }))
  }

  const getComponentConfiguration = (
    componentId: string
  ): ComponentParameterization | undefined => {
    return state.components.parameterized.find(p => p.componentId === componentId)
  }

  const updateComponentConfiguration = (
    componentId: string,
    config: Partial<ComponentParameterization>
  ) => {
    setState(prev => {
      const existing = prev.components.parameterized.find(p => p.componentId === componentId)

      if (existing) {
        return {
          ...prev,
          components: {
            ...prev.components,
            parameterized: prev.components.parameterized.map(p =>
              p.componentId === componentId ? { ...p, ...config } : p
            ),
          },
        }
      } else {
        return {
          ...prev,
          components: {
            ...prev.components,
            parameterized: [...prev.components.parameterized, { componentId, ...config }],
          },
        }
      }
    })
  }

  const addExtraComponent = (component: ExtraComponent) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        extra: [...prev.components.extra, component],
      },
    }))
  }

  // const updateExtraComponent = (index: number, component: ExtraComponent) => {
  //   setState(prev => ({
  //     ...prev,
  //     components: {
  //       ...prev.components,
  //       extra: prev.components.extra.map((c, i) => (i === index ? component : c)),
  //     },
  //   }))
  // }

  const removeExtraComponent = (index: number) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        extra: prev.components.extra.filter((_, i) => i !== index),
      },
    }))
  }

  const addComponentOverride = (componentId: string, override: ComponentOverride) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        overridden: [
          ...prev.components.overridden.filter(o => o.componentId !== componentId),
          override,
        ],
        // Remove from parameterized if overriding
        parameterized: prev.components.parameterized.filter(p => p.componentId !== componentId),
      },
    }))
  }

  const removeComponentOverride = (componentId: string) => {
    setState(prev => ({
      ...prev,
      components: {
        ...prev.components,
        overridden: prev.components.overridden.filter(o => o.componentId !== componentId),
      },
    }))
  }

  const getComponentOverride = (componentId: string): ComponentOverride | undefined => {
    return state.components.overridden.find(o => o.componentId === componentId)
  }

  const requiresConfiguration = (component: PriceComponent): boolean => {
    // Component requires configuration if it has configurable options and isn't overridden
    const isOverridden = state.components.overridden.some(o => o.componentId === component.id)
    if (isOverridden) return false

    const feeType = component.fee.fee
    // Rate components with multiple billing periods require configuration
    if (feeType === 'rate' && component.fee.data.rates?.length > 1) return true
    // Slot components always require configuration for seat count
    if (feeType === 'slot') return true
    // Capacity components with thresholds require configuration
    if (feeType === 'capacity' && component.fee.data.thresholds?.length > 0) return true

    return false
  }

  const isComponentConfigured = (component: PriceComponent): boolean => {
    if (!requiresConfiguration(component)) return true

    const isOverridden = state.components.overridden.some(o => o.componentId === component.id)
    if (isOverridden) return true

    const configuration = state.components.parameterized.find(p => p.componentId === component.id)
    if (!configuration) return false

    const feeType = component.fee.fee

    // Check that all required parameters are configured for each fee type
    if (feeType === 'rate' && component.fee.data.rates?.length > 1) {
      // Rate components with multiple periods need billing period selected
      return configuration.billingPeriod !== undefined
    }

    if (feeType === 'slot') {
      // Slot components need initial slot count
      const hasSlotCount = configuration.initialSlotCount !== undefined

      // If there are multiple billing periods, also need billing period selected
      if (component.fee.data.rates?.length > 1) {
        return hasSlotCount && configuration.billingPeriod !== undefined
      }

      return hasSlotCount
    }

    if (feeType === 'capacity' && component.fee.data.thresholds?.length > 0) {
      // Capacity components need committed capacity selected
      return configuration.committedCapacity !== undefined
    }

    return false
  }

  const getUnconfiguredComponents = () => {
    return (
      planPriceComponents?.filter(component => {
        const isExcluded = state.components.removed.includes(component.id)
        return !isExcluded && requiresConfiguration(component) && !isComponentConfigured(component)
      }) || []
    )
  }

  return (
    <div className="space-y-3">
      {/* Plan Components */}
      {planPriceComponents?.map(component => {
        const isExcluded = state.components.removed.includes(component.id)
        const configuration = getComponentConfiguration(component.id)
        const override = getComponentOverride(component.id)
        const isConfigured = isComponentConfigured(component)
        const isOverridden = !!override
        const isEditing = editingComponentId === component.id
        const needsConfiguration =
          requiresConfiguration(component) && !isConfigured && !isOverridden && !isExcluded

        return (
          <CompactPriceComponentCard
            key={component.id}
            component={component}
            isExcluded={isExcluded}
            isConfigured={isConfigured}
            isOverridden={isOverridden}
            isEditing={isEditing}
            needsConfiguration={needsConfiguration}
            configuration={configuration}
            override={override}
            currency={currency}
            onToggleExclude={() => toggleComponentRemoval(component.id)}
            onStartEdit={() => setEditingComponentId(component.id)}
            onEndEdit={() => setEditingComponentId(null)}
            onUpdateConfiguration={config => updateComponentConfiguration(component.id, config)}
            onStartOverride={() => setOverrideComponentId(component.id)}
            onRemoveOverride={() => removeComponentOverride(component.id)}
          />
        )
      })}

      {/* Extra Components */}
      {state.components.extra.map((extraComponent, index) => (
        <ExtraComponentCard
          key={`extra-${index}`}
          component={extraComponent}
          index={index}
          currency={currency}
          onEdit={() => setEditExtraIndex(index)}
          onRemove={() => removeExtraComponent(index)}
        />
      ))}

      {/* Add Fee Button */}
      <Button
        type="button"
        variant="outline"
        className="w-full border-dashed"
        onClick={() => setShowAddFeeModal(true)}
      >
        <Plus className="h-4 w-4 mr-2" />
        Add a fee
      </Button>

      {!planPriceComponents?.length && !state.components.extra.length && (
        <span className="text-muted-foreground">No price components</span>
      )}

      {/* Add Fee Modal */}
      {showAddFeeModal && (
        <AddFeeModal
          onClose={() => setShowAddFeeModal(false)}
          onAdd={addExtraComponent}
          currency={currency}
        />
      )}

      {/* Override Component Modal */}
      {overrideComponentId && (
        <OverrideFeeModal
          componentId={overrideComponentId}
          originalComponent={planPriceComponents?.find(c => c.id === overrideComponentId)}
          existingOverride={getComponentOverride(overrideComponentId)}
          onClose={() => setOverrideComponentId(null)}
          onSave={override => {
            addComponentOverride(overrideComponentId, override)
            setOverrideComponentId(null)
          }}
          currency={currency}
        />
      )}

      {/* Edit Extra Component Modal */}
      {editExtraIndex !== null && (
        <AddFeeModal
          onClose={() => setEditExtraIndex(null)}
          onAdd={component => {
            setState(prev => ({
              ...prev,
              components: {
                ...prev.components,
                extra: prev.components.extra.map((c, i) => (i === editExtraIndex ? component : c)),
              },
            }))
            setEditExtraIndex(null)
          }}
          currency={currency}
          initialValues={state.components.extra[editExtraIndex]}
          isEditing={true}
        />
      )}
    </div>
  )
}

interface CompactPriceComponentCardProps {
  component: PriceComponent
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
  const getFeeTypeIcon = (fee: string) => {
    switch (fee) {
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

  const getFeeTypeDisplay = (fee: string) => {
    switch (fee) {
      case 'rate':
        return 'Fixed'
      case 'usage':
        return 'Usage'
      case 'slot':
        const slotUnitName =
          component.fee.fee === 'slot' ? component.fee.data.slotUnitName || 'seat' : 'seat'
        return `Per ${slotUnitName}`
      case 'capacity':
        return 'Capacity'
      case 'oneTime':
        return 'One-time'
      case 'extraRecurring':
        return 'Recurring'
      default:
        return fee
    }
  }

  const getPrice = () => {
    // If overridden, show override price
    if (isOverridden && override?.fee) {
      return getPriceFromFee(override.fee)
    }

    // Otherwise show configured or default price
    if (
      (component.fee.fee === 'rate' || component.fee.fee === 'slot') &&
      component.fee.data.rates?.length > 0
    ) {
      if (configuration?.billingPeriod !== undefined) {
        const rate = component.fee.data.rates.find(r => {
          switch (configuration.billingPeriod) {
            case SharedBillingPeriod.MONTHLY:
              return r.term === 'MONTHLY'
            case SharedBillingPeriod.QUARTERLY:
              return r.term === 'QUARTERLY'
            case SharedBillingPeriod.ANNUAL:
              return r.term === 'ANNUAL'
            default:
              return false
          }
        })
        if (rate) return rate.price
      }
      return component.fee.data.rates[0].price
    }

    if (component.fee.fee === 'capacity' && component.fee.data.thresholds?.length > 0) {
      if (configuration?.committedCapacity !== undefined) {
        const threshold = component.fee.data.thresholds.find(
          t => BigInt(t.includedAmount) === configuration.committedCapacity
        )
        if (threshold) return threshold.price
      }
      return component.fee.data.thresholds[0].price
    }
    if (component.fee.fee === 'usage' && component.fee.data.model.model === 'per_unit') {
      return component.fee.data.model.data.unitPrice
    }
    if (component.fee.fee === 'oneTime') {
      return component.fee.data.unitPrice
    }
    if (component.fee.fee === 'extraRecurring') {
      return component.fee.data.unitPrice
    }
    return '0'
  }

  const getPriceFromFee = (fee: any): string => {
    if (fee.data?.unitPrice) {
      return fee.data.unitPrice
    }
    return '0'
  }

  const formatPrice = (price: string | number) => {
    const amount = typeof price === 'string' ? parseFloat(price || '0') : price
    return amount.toLocaleString(undefined, {
      style: 'currency',
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })
  }

  const canConfigure = () => {
    if (isOverridden) return false
    const feeType = component.fee.fee
    if (feeType === 'rate' && component.fee.data.rates?.length > 1) return true
    if (feeType === 'slot') return true
    if (feeType === 'capacity' && component.fee.data.thresholds?.length > 0) return true
    return false
  }

  const getPriceDisplay = () => {
    // If component needs configuration, show warning instead of price
    if (needsConfiguration && !isExcluded) {
      return (
        <div className="text-right">
          <div className="text-xs font-medium text-red-700">Configuration required</div>
        </div>
      )
    }

    const unitPrice = parseFloat(getPrice())
    const billingPeriodLabel = getSchemaComponentBillingPeriodLabel(component, configuration)

    // Check if this is an overridden component with quantity
    if (isOverridden && override?.fee?.data?.quantity && override.fee.data.quantity > 1) {
      const quantity = override.fee.data.quantity
      const totalPrice = unitPrice * quantity
      return (
        <div className="text-right">
          <div className="flex items-center justify-end gap-1">
            <span>
              {formatPrice(unitPrice)} × {quantity}
            </span>
            <Badge variant="secondary" size="sm">
              {billingPeriodLabel}
            </Badge>
          </div>
          <div className="text-xs text-muted-foreground">{formatPrice(totalPrice)}</div>
        </div>
      )
    }

    // Check if this is a slot component with configured slot count
    if (
      component.fee.fee === 'slot' &&
      configuration?.initialSlotCount &&
      configuration.initialSlotCount > 1
    ) {
      const slotCount = configuration.initialSlotCount
      const totalPrice = unitPrice * slotCount
      return (
        <div className="text-right">
          <div className="flex items-center justify-end gap-1">
            <span>
              {formatPrice(unitPrice)} × {slotCount} slots
            </span>
            <Badge variant="secondary" size="sm">
              {billingPeriodLabel}
            </Badge>
          </div>
          <div className="text-xs text-muted-foreground">{formatPrice(totalPrice)}</div>
        </div>
      )
    }

    return (
      <div className="text-right">
        <div className="flex items-center justify-end gap-1">
          <span>{formatPrice(unitPrice)}</span>
          <Badge variant="secondary" size="sm">
            {billingPeriodLabel}
          </Badge>
        </div>
      </div>
    )
  }

  const renderConfiguration = () => {
    if (!isEditing || isExcluded) return null

    const feeType = component.fee.fee

    if (feeType === 'rate' && component.fee.data.rates?.length > 1) {
      return (
        <div className="space-y-2">
          <Label className="text-xs">Billing Period</Label>
          <Select
            value={configuration?.billingPeriod?.toString() || ''}
            onValueChange={value =>
              onUpdateConfiguration({ billingPeriod: parseInt(value) as SharedBillingPeriod })
            }
          >
            <SelectTrigger className="h-8 text-xs">
              <SelectValue placeholder="Select period" />
            </SelectTrigger>
            <SelectContent>
              {component.fee.data.rates.map(rate => {
                const period = mapTermToBillingPeriod(rate.term)
                return (
                  <SelectItem key={rate.term} value={period.toString()}>
                    <div className="flex justify-between items-center w-full">
                      <span>{rate.term.toLowerCase()}</span>
                      <span className="text-muted-foreground ml-2">{formatPrice(rate.price)}</span>
                    </div>
                  </SelectItem>
                )
              })}
            </SelectContent>
          </Select>
        </div>
      )
    }

    if (feeType === 'slot') {
      return (
        <div className="space-y-2">
          {component.fee.data.rates?.length > 1 && (
            <>
              <Label className="text-xs">Billing Period</Label>
              <Select
                value={configuration?.billingPeriod?.toString() || ''}
                onValueChange={value =>
                  onUpdateConfiguration({ billingPeriod: parseInt(value) as SharedBillingPeriod })
                }
              >
                <SelectTrigger className="h-8 text-xs">
                  <SelectValue placeholder="Select period" />
                </SelectTrigger>
                <SelectContent>
                  {component.fee.data.rates.map(rate => {
                    const period = mapTermToBillingPeriod(rate.term)
                    return (
                      <SelectItem key={rate.term} value={period.toString()}>
                        <div className="flex justify-between items-center w-full">
                          <span>{rate.term.toLowerCase()}</span>
                          <span className="text-muted-foreground ml-2">
                            {formatPrice(rate.price)}/
                            {component.fee.fee === 'slot'
                              ? component.fee.data.slotUnitName || 'seat'
                              : 'seat'}
                          </span>
                        </div>
                      </SelectItem>
                    )
                  })}
                </SelectContent>
              </Select>
            </>
          )}
          <Label className="text-xs">
            Initial{' '}
            {component.fee.fee === 'slot' ? component.fee.data.slotUnitName || 'Seats' : 'Seats'}
          </Label>
          <Input
            type="number"
            min="0"
            className="h-8 text-xs"
            placeholder={`Number of ${component.fee.fee === 'slot' ? component.fee.data.slotUnitName || 'seats' : 'seats'}`}
            value={configuration?.initialSlotCount || ''}
            onChange={e => {
              const value = e.target.value ? parseInt(e.target.value) : undefined
              onUpdateConfiguration({ initialSlotCount: value })
            }}
          />
        </div>
      )
    }

    if (feeType === 'capacity' && component.fee.data.thresholds?.length > 0) {
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
              {component.fee.data.thresholds.map(threshold => (
                <SelectItem key={threshold.includedAmount} value={threshold.includedAmount}>
                  <div className="flex justify-between items-center w-full">
                    <span>{threshold.includedAmount} included</span>
                    <span className="text-muted-foreground ml-2">
                      {formatPrice(threshold.price)}
                    </span>
                  </div>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )
    }

    return null
  }

  return (
    <Card
      className={`transition-all duration-200 hover:shadow-sm ${
        isExcluded
          ? 'opacity-50 bg-muted/30'
          : needsConfiguration
            ? 'bg-card  text-card-foreground'
            : ''
      }`}
    >
      <CardHeader className="p-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3 flex-1">
            <div className="text-muted-foreground">{getFeeTypeIcon(component.fee.fee)}</div>
            <div className="flex-1">
              <div className="flex items-center gap-2">
                <CardTitle className={cn('text-sm font-medium', isExcluded && 'line-through')}>
                  {isOverridden && override?.name ? override.name : component.name}
                </CardTitle>
                <Badge variant="outline" size="sm">
                  {getFeeTypeDisplay(component.fee.fee)}
                </Badge>

                {isOverridden && (
                  <Badge variant="destructive" size="sm">
                    Custom Price
                  </Badge>
                )}
              </div>
            </div>
            <div className={cn('text-sm font-medium', isExcluded && 'line-through')}>
              {getPriceDisplay()}
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
                  disabled
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
            {configuration?.billingPeriod !== undefined && (
              <span>Billing: {getBillingPeriodLabel(configuration.billingPeriod)}</span>
            )}
            {configuration?.initialSlotCount !== undefined && (
              <span>
                {configuration.billingPeriod !== undefined && ' • '}
                {component.fee.fee === 'slot'
                  ? component.fee.data.slotUnitName || 'Seats'
                  : 'Seats'}
                : {configuration.initialSlotCount}
              </span>
            )}
            {configuration?.committedCapacity !== undefined && (
              <span>
                {(configuration.billingPeriod !== undefined ||
                  configuration.initialSlotCount !== undefined) &&
                  ' • '}
                Capacity: {configuration.committedCapacity.toString()}
              </span>
            )}
          </div>
        </CardContent>
      )}
    </Card>
  )
}

// Extra Component Card
interface ExtraComponentCardProps {
  component: ExtraComponent
  index: number
  currency: string
  onEdit: () => void
  onRemove: () => void
}

const ExtraComponentCard = ({ component, currency, onEdit, onRemove }: ExtraComponentCardProps) => {
  const formatPrice = (price: string | number) => {
    const amount = typeof price === 'string' ? parseFloat(price || '0') : price
    return amount.toLocaleString(undefined, {
      style: 'currency',
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })
  }

  const unitPrice = parseFloat(component.fee?.data?.unitPrice || '0')
  const quantity = component.fee?.data?.quantity || 1
  const totalPrice = unitPrice * quantity

  const getPriceDisplay = () => {
    const billingPeriodLabel = getExtraComponentBillingPeriodLabel(component.fee?.fee)

    if (quantity > 1) {
      return (
        <div className="text-sm font-medium text-right">
          <div className="flex items-center justify-end gap-1">
            <span>
              {formatPrice(unitPrice)} × {quantity}
            </span>
            <Badge variant="secondary" size="sm">
              {billingPeriodLabel}
            </Badge>
          </div>
          <div className="text-xs text-muted-foreground">{formatPrice(totalPrice)}</div>
        </div>
      )
    }
    return (
      <div className="text-sm font-medium text-right">
        <div className="flex items-center justify-end gap-1">
          <span>{formatPrice(unitPrice)}</span>
          <Badge variant="secondary" size="sm">
            {billingPeriodLabel}
          </Badge>
        </div>
      </div>
    )
  }

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
            {getPriceDisplay()}
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

// Add Fee Modal - Enhanced schema for complete fee types
const addFeeSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  feeType: z.enum(['rate', 'oneTime', 'extraRecurring', 'slot', 'capacity']),
  unitPrice: z.string().min(1, 'Price is required'),
  quantity: z.number().positive().int().default(1),
  // Slot-specific fields
  slotUnitName: z.string().optional(),
  minSlots: z.number().positive().int().optional(),
  maxSlots: z.number().positive().int().optional(),
  // Capacity-specific fields
  includedAmount: z.string().optional(),
  overageRate: z.string().optional(),
  metricId: z.string().optional(),
  // Rate-specific fields
  billingType: z.enum(['ARREAR', 'ADVANCE']).default('ARREAR'),
})

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
  const form = useForm<z.infer<typeof addFeeSchema>>({
    resolver: zodResolver(addFeeSchema),
    defaultValues: {
      name: initialValues?.name || '',
      feeType: (initialValues?.fee?.fee as any) || 'oneTime',
      unitPrice: initialValues?.fee?.data?.unitPrice || '',
      quantity: initialValues?.fee?.data?.quantity || 1,
    },
  })

  const onSubmit = (values: z.infer<typeof addFeeSchema>) => {
    let componentData: any = {
      unitPrice: values.unitPrice,
      quantity: values.quantity,
    }

    // Add type-specific data
    if (values.feeType === 'slot') {
      componentData = {
        ...componentData,
        slotUnitName: values.slotUnitName || 'seat',
        minSlots: values.minSlots,
        maxSlots: values.maxSlots,
      }
    } else if (values.feeType === 'capacity') {
      componentData = {
        ...componentData,
        includedAmount: values.includedAmount,
        overageRate: values.overageRate,
        metricId: values.metricId,
      }
    } else if (values.feeType === 'extraRecurring') {
      componentData = {
        ...componentData,
        billingType: values.billingType,
      }
    }

    const component: ExtraComponent = {
      name: values.name,
      fee: {
        fee: values.feeType,
        data: componentData,
      },
    }
    onAdd(component)
    onClose()
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    e.stopPropagation()
    form.handleSubmit(onSubmit)(e)
  }

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[425px]">
        <DialogHeader>
          <DialogTitle>{isEditing ? 'Edit Custom Fee' : 'Add Custom Fee'}</DialogTitle>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={handleSubmit} className="space-y-4">
            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Fee Name</FormLabel>
                  <FormControl>
                    <Input placeholder="e.g., Setup Fee, Custom Service" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="feeType"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Fee Type</FormLabel>
                  <Select onValueChange={field.onChange} defaultValue={field.value}>
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue placeholder="Select fee type" />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="oneTime">One-time Fee</SelectItem>
                      <SelectItem value="extraRecurring">Recurring Fee</SelectItem>
                      <SelectItem value="rate">Fixed Rate</SelectItem>
                      <SelectItem value="slot">Per Slot/Seat</SelectItem>
                      <SelectItem disabled value="capacity">
                        Capacity-based
                      </SelectItem>
                      <SelectItem disabled value="usage">
                        Usage-based
                      </SelectItem>
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="unitPrice"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Price ({currency})</FormLabel>
                  <FormControl>
                    <Input type="number" step="0.01" min="0" placeholder="0.00" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="quantity"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Quantity</FormLabel>
                  <FormControl>
                    <Input
                      type="number"
                      min="1"
                      placeholder="1"
                      {...field}
                      onChange={e => field.onChange(parseInt(e.target.value) || 1)}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            {/* Slot-specific fields */}
            {form.watch('feeType') === 'slot' && (
              <>
                <FormField
                  control={form.control}
                  name="slotUnitName"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Slot Unit Name</FormLabel>
                      <FormControl>
                        <Input placeholder="e.g., seat, user, license" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <div className="grid grid-cols-2 gap-4">
                  <FormField
                    control={form.control}
                    name="minSlots"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Min Slots</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min="1"
                            {...field}
                            onChange={e => field.onChange(parseInt(e.target.value) || undefined)}
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <FormField
                    control={form.control}
                    name="maxSlots"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Max Slots</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min="1"
                            {...field}
                            onChange={e => field.onChange(parseInt(e.target.value) || undefined)}
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>
              </>
            )}

            {/* Capacity-specific fields */}
            {form.watch('feeType') === 'capacity' && (
              <>
                <FormField
                  control={form.control}
                  name="includedAmount"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Included Amount</FormLabel>
                      <FormControl>
                        <Input type="number" placeholder="e.g., 1000" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="overageRate"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Overage Rate ({currency})</FormLabel>
                      <FormControl>
                        <Input type="number" step="0.01" placeholder="0.00" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="metricId"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Metric ID</FormLabel>
                      <FormControl>
                        <Input placeholder="metric_id" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </>
            )}

            {/* Extra recurring specific fields */}
            {form.watch('feeType') === 'extraRecurring' && (
              <FormField
                control={form.control}
                name="billingType"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Billing Type</FormLabel>
                    <Select onValueChange={field.onChange} defaultValue={field.value}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder="Select billing type" />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="ARREAR">In Arrears</SelectItem>
                        <SelectItem value="ADVANCE">In Advance</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}

            <div className="flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit">{isEditing ? 'Save Changes' : 'Add Fee'}</Button>
            </div>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}

// Override Fee Modal
const overrideFeeSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  unitPrice: z.string().min(1, 'Price is required'),
})

interface OverrideFeeModalProps {
  componentId: string
  originalComponent?: PriceComponent
  existingOverride?: ComponentOverride
  onClose: () => void
  onSave: (override: ComponentOverride) => void
  currency: string
}

const OverrideFeeModal = ({
  componentId,
  originalComponent,
  existingOverride,
  onClose,
  onSave,
  currency,
}: OverrideFeeModalProps) => {
  const form = useForm<z.infer<typeof overrideFeeSchema>>({
    resolver: zodResolver(overrideFeeSchema),
    defaultValues: {
      name: existingOverride?.name || originalComponent?.name || '',
      unitPrice: existingOverride?.fee?.data?.unitPrice || '',
    },
  })

  const onSubmit = (values: z.infer<typeof overrideFeeSchema>) => {
    const override: ComponentOverride = {
      componentId,
      name: values.name,
      fee: {
        // Simplified override - could be extended based on original component type
        fee: 'oneTime',
        data: {
          unitPrice: values.unitPrice,
          quantity: 1,
        },
      },
    }
    onSave(override)
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    e.stopPropagation()
    form.handleSubmit(onSubmit)(e)
  }

  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[425px]">
        <DialogHeader>
          <DialogTitle>Override Component Pricing</DialogTitle>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={handleSubmit} className="space-y-4">
            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Component Name</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="unitPrice"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Override Price ({currency})</FormLabel>
                  <FormControl>
                    <Input type="number" step="0.01" min="0" placeholder="0.00" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <div className="flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit">Save Override</Button>
            </div>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}

// Helper functions

// Fee mapping functions
const mapExtraComponentToSubscriptionComponent = (
  component: ExtraComponent
): SubscriptionComponentNewInternal => {
  const subscriptionComponent = new SubscriptionComponentNewInternal({
    name: component.name,
    period: SubscriptionFeeBillingPeriod.ONE_TIME, // Default for extra components
  })

  const fee = new SubscriptionFee()

  if (component.fee.fee === 'oneTime') {
    fee.fee = {
      case: 'oneTime',
      value: new SubscriptionFee_OneTimeSubscriptionFee({
        rate: component.fee.data.unitPrice,
        quantity: component.fee.data.quantity || 1,
        total: (
          parseFloat(component.fee.data.unitPrice) * (component.fee.data.quantity || 1)
        ).toString(),
      }),
    }
  } else if (component.fee.fee === 'extraRecurring') {
    fee.fee = {
      case: 'recurring',
      value: new SubscriptionFee_ExtraRecurringSubscriptionFee({
        rate: component.fee.data.unitPrice,
        quantity: component.fee.data.quantity || 1,
        total: (
          parseFloat(component.fee.data.unitPrice) * (component.fee.data.quantity || 1)
        ).toString(),
        billingType: Fee_BillingType.ARREAR, // Default
      }),
    }
  } else if (component.fee.fee === 'rate') {
    fee.fee = {
      case: 'rate',
      value: new SubscriptionFee_RateSubscriptionFee({
        rate: component.fee.data.unitPrice,
      }),
    }
  } else if (component.fee.fee === 'slot') {
    fee.fee = {
      case: 'slot',
      value: new SubscriptionFee_SlotSubscriptionFee({
        unit: component.fee.data.slotUnitName || 'seat',
        unitRate: component.fee.data.unitPrice,
        minSlots: component.fee.data.minSlots,
        maxSlots: component.fee.data.maxSlots,
      }),
    }
  } else if (component.fee.fee === 'capacity') {
    fee.fee = {
      case: 'capacity',
      value: new SubscriptionFee_CapacitySubscriptionFee({
        rate: component.fee.data.unitPrice,
        included: BigInt(component.fee.data.includedAmount || '0'),
        overageRate: component.fee.data.overageRate || '0',
        metricId: component.fee.data.metricId || '',
      }),
    }
  }

  subscriptionComponent.fee = fee
  return subscriptionComponent
}

const mapOverrideComponentToSubscriptionComponent = (
  override: ComponentOverride
): SubscriptionComponentNewInternal => {
  const subscriptionComponent = new SubscriptionComponentNewInternal({
    priceComponentId: override.componentId,
    name: override.name,
    period: SubscriptionFeeBillingPeriod.ONE_TIME, // Default for overrides
  })

  const fee = new SubscriptionFee()

  if (override.fee.fee === 'oneTime') {
    fee.fee = {
      case: 'oneTime',
      value: new SubscriptionFee_OneTimeSubscriptionFee({
        rate: override.fee.data.unitPrice,
        quantity: override.fee.data.quantity || 1,
        total: (
          parseFloat(override.fee.data.unitPrice) * (override.fee.data.quantity || 1)
        ).toString(),
      }),
    }
  } else if (override.fee.fee === 'extraRecurring') {
    fee.fee = {
      case: 'recurring',
      value: new SubscriptionFee_ExtraRecurringSubscriptionFee({
        rate: override.fee.data.unitPrice,
        quantity: override.fee.data.quantity || 1,
        total: (
          parseFloat(override.fee.data.unitPrice) * (override.fee.data.quantity || 1)
        ).toString(),
        billingType: Fee_BillingType.ARREAR,
      }),
    }
  } else if (override.fee.fee === 'rate') {
    fee.fee = {
      case: 'rate',
      value: new SubscriptionFee_RateSubscriptionFee({
        rate: override.fee.data.unitPrice,
      }),
    }
  }

  subscriptionComponent.fee = fee
  return subscriptionComponent
}

// Export mapping functions for use in StepReviewAndCreate
export { mapExtraComponentToSubscriptionComponent, mapOverrideComponentToSubscriptionComponent }
