import { disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Separator,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { PencilIcon, Trash2Icon } from 'lucide-react'
import { useNavigate, useParams } from 'react-router-dom'

import { LocalId } from '@/components/LocalId'
import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { PricingDetailsView } from '@/features/plans/pricecomponents/components/PricingDetailsView'
import {
  feeTypeIcon,
  feeTypeToHuman,
} from '@/features/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { formatCadence } from '@/lib/mapping/prices'
import {
  getAddOn,
  listAddOns,
  removeAddOn,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

export const AddonDetailPanel = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const showConfirmationModal = useConfirmationModal()
  const { addonId } = useParams<{ addonId: string }>()

  const addonQuery = useQuery(
    getAddOn,
    addonId ? { addOnId: addonId } : disableQuery
  )

  const removeMutation = useMutation(removeAddOn, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: [listAddOns.service.typeName],
      })
      navigate('..')
    },
  })

  const addOn = addonQuery.data?.addOn
  const isLoading = addonQuery.isLoading

  const componentFeeType = addOn ? feeTypeEnumToComponentFeeType(addOn.feeType) : undefined
  const Icon = componentFeeType ? feeTypeIcon(componentFeeType) : undefined
  const feeLabel = componentFeeType ? feeTypeToHuman(componentFeeType) : undefined
  const cadence = addOn?.price ? formatCadence(addOn.price.cadence) : undefined
  const currency = addOn?.price?.currency

  const handleRemove = () => {
    if (!addOn) return
    showConfirmationModal(() =>
      removeMutation.mutate({ addOnId: addOn.id })
    )
  }

  return (
    <Sheet open={true} onOpenChange={() => navigate('..')}>
      <SheetContent size="medium">
        <SheetHeader className="pb-2">
          <SheetTitle>Add-on Details</SheetTitle>
          <Separator />
        </SheetHeader>

        {isLoading && (
          <div className="flex flex-col gap-4 py-4">
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-64" />
            <Skeleton className="h-4 w-32" />
          </div>
        )}

        {addOn && !isLoading && (
          <div className="flex flex-col gap-6 py-4">
            <section className="flex flex-col gap-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  {Icon && <Icon className="w-5 h-5 text-muted-foreground" />}
                  <h3 className="text-lg font-semibold">{addOn.name}</h3>
                </div>
                <div className="flex items-center gap-1">
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => navigate(`../edit/${addOn.id}`)}
                  >
                    <PencilIcon size={16} />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="text-destructive hover:text-destructive"
                    onClick={handleRemove}
                  >
                    <Trash2Icon size={16} />
                  </Button>
                </div>
              </div>
              <LocalId localId={addOn.id} className="max-w-24" />
              {addOn.description && (
                <p className="text-sm text-muted-foreground">{addOn.description}</p>
              )}
            </section>

            <Separator />

            <section className="flex flex-col gap-3">
              <h3 className="text-sm font-medium text-muted-foreground">Pricing</h3>
              <div className="flex flex-col gap-2">
                <DetailRow label="Fee type">
                  {feeLabel && <Badge variant="secondary">{feeLabel}</Badge>}
                </DetailRow>
                {currency && (
                  <DetailRow label="Currency">
                    <span className="font-mono text-xs">{currency.toUpperCase()}</span>
                  </DetailRow>
                )}
                {cadence && (
                  <DetailRow label="Cadence">
                    <span>{cadence}</span>
                  </DetailRow>
                )}
              </div>
              {addOn.price && currency && (
                <div className="mt-2">
                  <PricingDetailsView prices={[addOn.price]} currency={currency} />
                </div>
              )}
            </section>

            <Separator />

            <section className="flex flex-col gap-3">
              <h3 className="text-sm font-medium text-muted-foreground">Settings</h3>
              <div className="flex flex-col gap-2">
                <DetailRow label="Self-service">
                  <span>{addOn.selfServiceable ? 'Yes' : 'No'}</span>
                </DetailRow>
                <DetailRow label="Max per subscription">
                  <span>
                    {addOn.maxInstancesPerSubscription
                      ? addOn.maxInstancesPerSubscription
                      : 'Unlimited'}
                  </span>
                </DetailRow>
              </div>
            </section>
          </div>
        )}
      </SheetContent>
    </Sheet>
  )
}

function DetailRow({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-sm text-muted-foreground w-36 shrink-0">{label}</span>
      <div className="text-sm">{children}</div>
    </div>
  )
}
