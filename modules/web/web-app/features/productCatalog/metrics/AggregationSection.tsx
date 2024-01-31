import { Badge, FormItem, Input, SelectGroup, SelectItem, SidePanel } from '@md/ui'
import { useWatch } from 'react-hook-form'

import { ControlledSelect } from '@/components/form'
import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

interface Props {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}

export const AggregationSection = ({ methods }: Props) => {
  return (
    <>
      <SidePanel.Separator />
      <SidePanel.Content>
        <div className="space-y-6 py-6">
          <FormItem
            name="aggregationType"
            label="Aggregation type"
            error={methods.formState.errors.aggregation?.aggregationType?.message}
          >
            <ControlledSelect
              {...methods.withControl('aggregation.aggregationType')}
              className="max-w-sm"
              placeholder="Select an aggregation type"
            >
              <SelectGroup className="text-sm text-slate-1100 py-2 pl-4">Standard</SelectGroup>
              <SelectItem value="COUNT">Count</SelectItem>
              <SelectItem value="COUNT_DISTINCT">Count Distinct</SelectItem>
              <SelectItem value="SUM">Sum</SelectItem>
              <SelectItem value="MEAN">Mean</SelectItem>
              <SelectItem value="MIN">Min</SelectItem>
              <SelectItem value="MAX">Max</SelectItem>
              <SelectItem value="LATEST">Latest</SelectItem>
              <SelectGroup className="text-sm text-slate-1100 py-2 pl-4">Advanced</SelectGroup>
              <SelectItem value="COMPOUND" disabled badge={<Badge>Soon</Badge>}>
                Compound
              </SelectItem>
              <SelectItem value="UNIQUE_PERSISTENT" disabled badge={<Badge>Soon</Badge>}>
                Unique (persistent)
              </SelectItem>
              <SelectItem value="GAUGE" disabled badge={<Badge>Soon</Badge>}>
                Gauge
              </SelectItem>
            </ControlledSelect>
          </FormItem>
          <AggregationData methods={methods} />
        </div>
      </SidePanel.Content>
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
        <FormItem
          name="aggregation.aggregationKey"
          label="Aggregation key"
          error={methods.formState.errors.aggregation?.aggregationKey?.message}
          hint="This property must be passed in the event dimensions"
        >
          <Input
            type="text"
            placeholder="some_property"
            {...methods.register('aggregation.aggregationKey')}
          />
        </FormItem>
      )}
    </>
  )
}
