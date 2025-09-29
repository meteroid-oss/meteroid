import {
  FormDescription,
  InputFormField,
} from '@md/ui'

import { AccordionPanel } from '@/components/AccordionPanel'
import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

interface UsageGroupKeyProps {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}

export const UsageGroupKeySection = ({ methods }: UsageGroupKeyProps) => {
  return (
    <>
      <AccordionPanel
        title={
          <div className="space-x-4 items-center flex pr-4">
            <h3>Usage Grouping</h3>
            <span className="text-xs text-muted-foreground">optional</span>
          </div>
        }
        defaultOpen={false}
      >
        <div className="space-y-6">
          <FormDescription>
            <p>
              Group usage by dynamic values from your events, such as cluster ID, project ID, or website ID.
              <br />
              This allows you to aggregate usage separately for each unique value of the specified field.
            </p>
          </FormDescription>
          <div className="space-y-4">
            <InputFormField
              name="usageGroupKey"
              label="Grouping Key"
              control={methods.control}
              placeholder="cluster_id"
              className="max-w-xs"
            />
            <FormDescription>
              The event property name to group usage by. Each unique value will be tracked separately.
            </FormDescription>
          </div>
        </div>
      </AccordionPanel>
    </>
  )
}