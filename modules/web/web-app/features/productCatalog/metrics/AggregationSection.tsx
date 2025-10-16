import {
  Badge,
  FormDescription,
  InputFormField,
  SelectFormField,
  SelectItem,
  Separator,
} from '@md/ui'
import { useWatch } from 'react-hook-form'

import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

interface Props {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}

export const AggregationSection = ({ methods }: Props) => {
  return (
    <>
      <div className="space-y-4">
        <div className="space-y-1">
          <h3 className="text-sm font-medium">Aggregation</h3>
          <p className="text-xs text-muted-foreground">
            Define how usage events should be aggregated for billing
          </p>
        </div>

        <div className="space-y-4 pl-4 border-l-2 border-muted">
          <SelectFormField
            name="aggregation.aggregationType"
            label="Aggregation type"
            control={methods.control}
            placeholder="Select an aggregation type"
            className="max-w-xs"
          >
            <SelectItem value="COUNT">Count</SelectItem>
            <SelectItem value="COUNT_DISTINCT" disabled>
              Count Distinct
            </SelectItem>
            <SelectItem value="SUM">Sum</SelectItem>
            <SelectItem value="MEAN">Mean</SelectItem>
            <SelectItem value="MIN">Min</SelectItem>
            <SelectItem value="MAX">Max</SelectItem>
            <SelectItem value="LATEST">Latest</SelectItem>
            <Separator />
            <SelectItem value="COMPOUND" disabled badge={<Badge variant="secondary">soon</Badge>}>
              Compound
            </SelectItem>
            <SelectItem
              value="UNIQUE_PERSISTENT"
              disabled
              badge={<Badge variant="secondary">soon</Badge>}
            >
              Unique (persistent)
            </SelectItem>
            <SelectItem value="GAUGE" disabled badge={<Badge variant="secondary">soon</Badge>}>
              Gauge
            </SelectItem>
          </SelectFormField>
          <AggregationData methods={methods} />
        </div>
      </div>
    </>
  )
}

interface AggregationDataProps {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}
const AggregationData = ({ methods }: AggregationDataProps) => {
  const aggregationType = useWatch({
    control: methods.control,
    name: 'aggregation.aggregationType',
  })

  return (
    <>
      {aggregationType && aggregationType !== 'COUNT' && (
        <div className="space-y-2">
          <InputFormField
            name="aggregation.aggregationKey"
            placeholder="some_property"
            label="Aggregation key"
            control={methods.control}
            className="max-w-xs"
          />
          <FormDescription>This property must be passed in the event dimensions</FormDescription>
        </div>
      )}
    </>
  )
}
