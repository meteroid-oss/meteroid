import { toJsonString } from '@bufbuild/protobuf'
import { Button, Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle, Skeleton } from '@md/ui'
import { toast } from 'sonner'

import { CustomerSelect } from '@/features/customers/CustomerSelect'
import {
  CheckAmountsFields,
  CheckOutcome,
  FeatureSelect,
  useCheckAmounts,
  useCheckSubject,
} from '@/features/entitlements/explain/CheckForm'
import { ComputationTrace } from '@/features/entitlements/explain/ComputationTrace'
import { EvaluateFor } from '@/features/entitlements/explain/EvaluateFor'
import { type Target, curlPost, isTargetReady, restCheckBody } from '@/features/entitlements/explain/checkRequest'
import { useExplain } from '@/features/entitlements/explain/useExplain'
import { env } from '@/lib/env'
import { copyToClipboard } from '@/lib/helpers'
import { CheckEntitlementResponseSchema } from '@/rpc/api/entitlements/v1/entitlements_pb'
import { type Feature } from '@/rpc/api/entitlements/v1/models_pb'

type Props = {
  /** Unset: the dialog picks the customer. */
  customer?: { id: string; name: string }
  feature?: Feature
  target?: Target
  onClose: () => void
}

export const ExplainDialog = ({ customer: fixed, feature: preset, target: initial, onClose }: Props) => {
  const subject = useCheckSubject({ customer: fixed, feature: preset, target: initial })
  const { customerId, customerName, feature, target } = subject
  const { methods, amounts, withRequested, withCount } = useCheckAmounts(feature)

  const check = useExplain(customerId ?? '', feature?.code, target, !!amounts, amounts)
  const data = customerId ? check.data : undefined

  const copyRequest = () => {
    if (!feature || !customerId) return
    copyToClipboard(curlPost(env.meteroidRestApiUri, customerId, restCheckBody(feature.code, target, amounts)), () =>
      toast.success('Copied')
    )
  }

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{feature ? feature.name : 'Evaluate'}</DialogTitle>
        </DialogHeader>

        <div className="flex flex-wrap items-center gap-2">
          {!fixed && (
            <CustomerSelect
              value={subject.pickedId}
              onChange={subject.pickCustomer}
              placeholder="Customer"
              triggerClassName="h-8 w-56 text-[13px]"
            />
          )}
          {!preset && (
            <FeatureSelect
              value={subject.featureId}
              onChange={subject.setFeatureId}
              features={subject.features}
              className="h-8 w-56 text-[13px]"
            />
          )}
          {customerId && (
            <EvaluateFor
              customerId={customerId}
              customerName={customerName}
              value={target}
              onChange={subject.setTarget}
            />
          )}
          {data && feature && (
            <div className="ml-auto">
              <CheckOutcome data={data} feature={feature} />
            </div>
          )}
        </div>

        {withRequested && (
          <CheckAmountsFields
            methods={methods}
            withCount={withCount}
            className="max-w-md"
            inputClassName="h-8 text-[13px]"
          />
        )}

        {feature && customerId && isTargetReady(target) && check.isLoading && <Skeleton className="h-32 w-full" />}
        {customerId && check.error && <p className="text-sm text-destructive">{check.error.rawMessage}</p>}
        {data && feature && customerId && (
          <ComputationTrace
            data={data}
            feature={feature}
            customerId={customerId}
            customerName={customerName}
            target={target}
          />
        )}

        {data && feature && (
          <DialogFooter className="gap-2">
            <Button size="sm" variant="outline" onClick={copyRequest}>
              Copy as API request
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                copyToClipboard(toJsonString(CheckEntitlementResponseSchema, data, { prettySpaces: 2 }), () =>
                  toast.success('Copied')
                )
              }
            >
              Copy debug JSON
            </Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  )
}
