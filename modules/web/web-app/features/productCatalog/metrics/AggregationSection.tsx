import {
  Badge,
  SelectFormField,
  SelectItem,
  Separator,
} from '@md/ui'
import { useWatch } from 'react-hook-form'

import { PropertyKeyFormField } from '@/features/productCatalog/metrics/PropertyKeyFormField'
import {
  SettingsFieldGrid,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

interface Props {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}

export const AggregationSection = ({ methods }: Props) => {
  return (
    <SettingsSection
      title="Aggregation"
      description="Define how usage events should be aggregated for billing"
    >
      <SettingsFieldGrid>
        <SelectFormField
          name="aggregation.aggregationType"
          label="Aggregation type"
          control={methods.control}
          placeholder="Select an aggregation type"
        >
          <SelectItem value="COUNT">Count</SelectItem>
          <SelectItem value="COUNT_DISTINCT">Count Distinct</SelectItem>
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
      </SettingsFieldGrid>
    </SettingsSection>
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
  const eventCode = useWatch({ control: methods.control, name: 'eventCode' })

  // SUM/MIN/MAX/MEAN operate on the numeric value of the key; COUNT_DISTINCT/LATEST
  // work on any type (e.g. COUNT_DISTINCT(user_id)). Only warn for the numeric ones.
  const numericExpected =
    aggregationType === 'SUM' ||
    aggregationType === 'MIN' ||
    aggregationType === 'MAX' ||
    aggregationType === 'MEAN'

  return (
    <>
      {aggregationType && aggregationType !== 'COUNT' && (
        <PropertyKeyFormField
          name="aggregation.aggregationKey"
          control={methods.control}
          eventCode={eventCode ?? ''}
          label="Aggregation key"
          placeholder="some_property"
          description="This property must be passed in the event dimensions"
          numericExpected={numericExpected}
        />
      )}
    </>
  )
}
