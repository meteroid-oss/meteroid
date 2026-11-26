import { toJsonString } from '@bufbuild/protobuf'
import { Button, Label, Skeleton, Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { CopyIcon } from 'lucide-react'
import { type ReactNode, useState } from 'react'
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
import {
  type CheckAmounts,
  type Target,
  curlGet,
  curlPost,
  formatGet,
  isTargetReady,
  restCheckBody,
  restCheckGet,
  restCheckPostPath,
} from '@/features/entitlements/explain/checkRequest'
import { useExplain } from '@/features/entitlements/explain/useExplain'
import { useResourceTypes } from '@/features/resources/hooks'
import { env } from '@/lib/env'
import { copyToClipboard } from '@/lib/helpers'
import { CheckEntitlementResponseSchema } from '@/rpc/api/entitlements/v1/entitlements_pb'

import type { Feature } from '@/rpc/api/entitlements/v1/models_pb'

type Run = {
  key: string
  customerId: string
  customerName: string
  feature: Feature
  target: Target
  amounts: CheckAmounts
}

/** Developers › Test a check: the check's request as the REST API takes it, and its answer with the trace. */
export const CheckPlayground = () => {
  const subject = useCheckSubject({})
  const { customerId, customerName, feature, target } = subject
  const { hasTypes } = useResourceTypes()
  const { methods, amounts, withRequested, withCount } = useCheckAmounts(feature)
  const [run, setRun] = useState<Run>()

  const check = useExplain(run?.customerId ?? '', run?.feature.code, run?.target ?? { kind: 'customer' }, !!run, run?.amounts)
  const data = check.data
  const ready = !!customerId && !!feature && isTargetReady(target) && !!amounts
  const body = feature && amounts ? restCheckBody(feature.code, target, amounts) : undefined

  const onRun = () => {
    if (!ready || !customerId || !feature || !amounts || !body) return
    const key = `${customerId}:${JSON.stringify(body)}`
    if (run?.key === key) void check.refetch()
    else setRun({ key, customerId, customerName, feature, target, amounts })
  }

  return (
    <div className="grid gap-5 lg:grid-cols-[minmax(0,340px)_minmax(0,1fr)]">
      <div className="flex flex-col gap-4 self-start rounded-lg border border-border bg-card p-4">
        <div className="space-y-1.5">
          <Label>Customer</Label>
          <CustomerSelect
            value={subject.pickedId}
            onChange={subject.pickCustomer}
            placeholder="Customer"
            triggerClassName="w-full"
          />
        </div>
        <div className="space-y-1.5">
          <Label>Feature</Label>
          <FeatureSelect
            value={subject.featureId}
            onChange={subject.setFeatureId}
            features={subject.features}
            className="w-full"
          />
        </div>
        {customerId && hasTypes && (
          <div className="space-y-1.5">
            <Label>Resource</Label>
            <EvaluateFor
              customerId={customerId}
              customerName={customerName}
              value={target}
              onChange={subject.setTarget}
            />
          </div>
        )}
        {withRequested && <CheckAmountsFields methods={methods} withCount={withCount} />}
        <Button type="button" size="sm" className="self-start" disabled={!ready || check.isFetching} onClick={onRun}>
          Run check
        </Button>
      </div>

      <div className="flex min-w-0 flex-col gap-4">
        {customerId && feature && body && (
          <section className="flex flex-col gap-2">
            <h3 className="text-sm font-semibold">Request</h3>
            <Tabs defaultValue="post" className="flex flex-col gap-2">
              <TabsList variant="ghost">
                <TabsTrigger value="post">POST</TabsTrigger>
                <TabsTrigger value="get">GET</TabsTrigger>
              </TabsList>
              <TabsContent value="post" className="mt-0">
                <Code copy={curlPost(env.meteroidRestApiUri, customerId, body)}>
                  {`POST ${restCheckPostPath(customerId)}\n\n${JSON.stringify(body, null, 2)}`}
                </Code>
              </TabsContent>
              <TabsContent value="get" className="mt-0">
                <Code copy={curlGet(env.meteroidRestApiUri, restCheckGet(customerId, feature.code, target, amounts))}>
                  {`GET ${formatGet(restCheckGet(customerId, feature.code, target, amounts))}`}
                </Code>
              </TabsContent>
            </Tabs>
          </section>
        )}

        {run && (
          <section className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h3 className="text-sm font-semibold">Response</h3>
              {data && (
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  hasIcon
                  onClick={() =>
                    copyToClipboard(toJsonString(CheckEntitlementResponseSchema, data, { prettySpaces: 2 }), () =>
                      toast.success('Copied')
                    )
                  }
                >
                  <CopyIcon size={14} /> Debug JSON
                </Button>
              )}
            </div>
            {check.isLoading && <Skeleton className="h-32 w-full" />}
            {check.error && <p className="text-sm text-destructive">{check.error.rawMessage}</p>}
            {data && (
              <>
                <CheckOutcome data={data} feature={run.feature} />
                <ComputationTrace
                  data={data}
                  feature={run.feature}
                  customerId={run.customerId}
                  customerName={run.customerName}
                  target={run.target}
                />
              </>
            )}
          </section>
        )}
      </div>
    </div>
  )
}

const Code = ({ copy, children }: { copy: string; children: ReactNode }) => (
  <div className="relative">
    <Button
      type="button"
      variant="ghost"
      size="icon"
      className="absolute right-1.5 top-1.5 h-7 w-7"
      aria-label="Copy"
      onClick={() => copyToClipboard(copy, () => toast.success('Copied'))}
    >
      <CopyIcon size={14} />
    </Button>
    <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded-md bg-muted p-3 pr-10 font-mono text-xs leading-relaxed text-foreground">
      {children}
    </pre>
  </div>
)
