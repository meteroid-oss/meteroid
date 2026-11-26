import {
  Alert,
  AlertDescription,
  Button,
  Label,
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { LockIcon, PlusIcon } from 'lucide-react'
import { useId, useState } from 'react'
import { Link } from 'react-router'

import { Combobox } from '@/components/Combobox'
import { ResourceDialog } from '@/features/resources/ResourceDialog'
import {
  attributesLabel,
  lower,
  resourceChip,
  resourceLabel,
  useCustomerResources,
  useResourceSubscriptions,
  useResourceTypes,
} from '@/features/resources/hooks'
import { useBasePath } from '@/hooks/useBasePath'

import type { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'

type Props = {
  customerId: string
  customerName: string
  planName?: string
  /** The plan's "Sold for": a warning when it differs, never a block. */
  soldForTypeId?: string
  value?: string
  onChange: (resourceId: string | undefined) => void
}

type Mode = 'customer' | 'resource'

/** "Subscribed for": the whole customer, or one of its stored resources. */
export const SubscribedFor = ({ customerId, planName, soldForTypeId, value, onChange }: Props) => {
  const id = useId()
  const { types, byId: typesById, hasTypes } = useResourceTypes()
  const { resources, byId } = useCustomerResources(customerId)
  const subscriptions = useResourceSubscriptions(customerId)
  const picked = value ? byId.get(value) : undefined
  const [chosenType, setTypeId] = useState<string>()
  const typeId = chosenType ?? picked?.resourceTypeId ?? soldForTypeId ?? types[0]?.id
  const [mode, setMode] = useState<Mode>(value || soldForTypeId ? 'resource' : 'customer')
  const [adding, setAdding] = useState(false)
  if (!hasTypes) return null

  const type = typeId ? typesById.get(typeId) : undefined
  const soldFor = soldForTypeId ? typesById.get(soldForTypeId) : undefined
  const pickType = !soldFor && types.length > 1
  const ofType = resources.filter(r => r.resourceTypeId === typeId && !r.archived)
  const existing = picked ? subscriptions.forResource(picked.id) : []
  const oldest = [...existing].sort((a, b) => a.startDate.localeCompare(b.startDate))[0]
  const mismatch = mode === 'customer' ? !!soldFor : !!picked && soldForTypeId !== picked.resourceTypeId

  const options = ofType.map(r => {
    const own = subscriptions.forResource(r.id).map(s => s.planName)
    const meta = [attributesLabel(r.attributes), own.length ? own.join(', ') : undefined].filter(Boolean).join(' · ')
    return {
      value: r.id,
      keywords: [r.keyValue, r.name ?? ''],
      label: (
        <span className="truncate">
          {resourceLabel(r)}
          {meta && <span className="text-muted-foreground"> · {meta}</span>}
        </span>
      ),
    }
  })

  return (
    <div className="flex flex-col gap-3">
      <RadioGroup
        value={mode}
        onValueChange={v => {
          setMode(v as Mode)
          if (v === 'customer') onChange(undefined)
        }}
        className="flex h-8 items-center gap-4"
      >
        <div className="flex items-center gap-2">
          <RadioGroupItem value="customer" id={`${id}-customer`} />
          <Label htmlFor={`${id}-customer`} className="cursor-pointer font-normal">
            Customer
          </Label>
        </div>
        <div className="flex items-center gap-2">
          <RadioGroupItem value="resource" id={`${id}-resource`} />
          <Label htmlFor={`${id}-resource`} className="cursor-pointer font-normal">
            {pickType ? 'Resource' : (type?.name ?? 'Resource')}
          </Label>
        </div>
      </RadioGroup>

      {mode === 'resource' && (
        <div className="flex flex-wrap items-center gap-2">
          {pickType && (
            <Select
              value={typeId}
              onValueChange={t => {
                setTypeId(t)
                onChange(undefined)
              }}
            >
              <SelectTrigger className="w-40">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {types.map(t => (
                  <SelectItem key={t.id} value={t.id}>
                    {t.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          <Combobox
            className="w-96 max-w-full"
            hasSearch
            options={options}
            value={value}
            onChange={onChange}
            placeholder={`Select a ${lower(type?.name ?? 'resource')}`}
            unit={lower(type?.pluralName ?? 'resources')}
            action={
              type && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  hasIcon
                  className="w-full justify-start"
                  onClick={() => setAdding(true)}
                >
                  <PlusIcon size={14} /> New {lower(type.name)}
                </Button>
              )
            }
          />
        </div>
      )}

      {picked && oldest && (
        <Alert variant="warning">
          <AlertDescription className="text-sm">
            {resourceLabel(picked)} already has {existing.map(s => s.planName).join(', ')}; {oldest.planName} keeps
            billing its usage.
          </AlertDescription>
        </Alert>
      )}
      {mismatch && (
        <Alert variant="warning">
          <AlertDescription className="text-sm">
            {planName ?? 'This plan'} is sold for {soldFor ? `a ${lower(soldFor.name)}` : 'the whole customer'}.
          </AlertDescription>
        </Alert>
      )}

      {adding && type && (
        <ResourceDialog
          customerId={customerId}
          type={type}
          withAttributes={false}
          onClose={saved => {
            setAdding(false)
            if (saved) onChange(saved.id)
          }}
        />
      )}
    </div>
  )
}

/** The value of the subscription page's "Subscribed for" row; fixed once active. */
export const SubscribedForDetail = ({ subscription }: { subscription: Subscription }) => {
  const basePath = useBasePath()
  const { byId: typesById } = useResourceTypes()
  const { byId } = useCustomerResources(subscription.resourceId ? subscription.customerId : undefined)
  if (!subscription.resourceId) return <>Customer</>
  const resource = byId.get(subscription.resourceId)
  const type = resource ? typesById.get(resource.resourceTypeId) : undefined
  const label = resourceChip(resource, type) ?? subscription.resourceId
  return (
    <span className="inline-flex items-center gap-1.5">
      {subscription.activatedAt && (
        <Tooltip>
          <TooltipTrigger asChild>
            <LockIcon className="h-3 w-3 text-muted-foreground" aria-label="Fixed once active" />
          </TooltipTrigger>
          <TooltipContent>Fixed once active</TooltipContent>
        </Tooltip>
      )}
      {type ? (
        <Link
          className="font-medium text-brand hover:underline"
          to={`${basePath}/customers/${subscription.customerId}?tab=resources-${type.code}`}
        >
          {label}
        </Link>
      ) : (
        label
      )}
    </span>
  )
}

/** What the subscription is sold with as it stands, under the subscription page's "Subscribed for". */
export const SubscribedForWarnings = ({
  subscription,
  warnings,
  metricsNotAttributed,
  plan,
}: {
  subscription: Subscription
  warnings: string[]
  metricsNotAttributed: string[]
  plan?: { soldForResourceTypeId?: string }
}) => {
  const { byId: typesById } = useResourceTypes()
  const { byId } = useCustomerResources(subscription.resourceId ? subscription.customerId : undefined)
  const resource = subscription.resourceId ? byId.get(subscription.resourceId) : undefined
  const type = resource ? typesById.get(resource.resourceTypeId) : undefined
  const soldFor = plan?.soldForResourceTypeId ? typesById.get(plan.soldForResourceTypeId) : undefined
  const mismatch = !!plan && warnings.includes('plan_sold_for_mismatch')
  const notAttributed = warnings.includes('metric_not_attributed') && metricsNotAttributed.length > 0
  if (!mismatch && !notAttributed) return null
  return (
    <div className="flex flex-col gap-1 py-2">
      {mismatch && (
        <p className="text-xs text-warning">
          The plan is sold for {soldFor ? `a ${lower(soldFor.name)}` : 'the whole customer'}.
        </p>
      )}
      {notAttributed && (
        <p className="text-xs text-warning">
          <MetricsNotAttributed codes={metricsNotAttributed} type={type} />
        </p>
      )}
    </div>
  )
}

/** The review step's "Subscribed for", and what its target is accepted with, as warnings. */
export const SubscribedForSummary = ({
  customerId,
  customerName,
  resourceId,
  soldForTypeId,
  metricsNotAttributed = [],
}: {
  customerId: string
  customerName: string
  resourceId?: string
  soldForTypeId?: string
  metricsNotAttributed?: string[]
}) => {
  const { byId: typesById, hasTypes } = useResourceTypes()
  const { byId } = useCustomerResources(customerId)
  if (!hasTypes) return null
  const resource = resourceId ? byId.get(resourceId) : undefined
  const soldFor = soldForTypeId ? typesById.get(soldForTypeId) : undefined
  return (
    <div className="flex flex-col gap-1">
      <div className="text-xs text-muted-foreground">Subscribed for</div>
      <div className="text-sm font-medium">
        {resource
          ? `${typesById.get(resource.resourceTypeId)?.name ?? ''} ${resourceLabel(resource)}`
          : `${customerName} (whole customer)`}
      </div>
      {resource?.resourceTypeId !== soldForTypeId && (
        <p className="text-xs text-warning">
          The plan is sold for {soldFor ? `a ${lower(soldFor.name)}` : 'the whole customer'}.
        </p>
      )}
      {resource && metricsNotAttributed.length > 0 && (
        <p className="text-xs text-warning">
          <MetricsNotAttributed
            codes={metricsNotAttributed}
            type={typesById.get(resource.resourceTypeId)}
          />
        </p>
      )}
    </div>
  )
}

/** A sale for a resource bills nothing of the metrics not attributed to its type. */
export const MetricsNotAttributed = ({
  codes,
  type,
}: {
  codes: string[]
  type?: { name: string; pluralName: string }
}) => (
  <>
    {codes.join(', ')} {codes.length > 1 ? 'are' : 'is'} not attributed to{' '}
    {lower(type?.pluralName ?? 'resources')}: never billed for a {lower(type?.name ?? 'resource')}.
  </>
)
