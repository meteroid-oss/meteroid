import { useWatch } from 'react-hook-form'

import { PropertyKeyFormField } from '@/features/productCatalog/metrics/PropertyKeyFormField'
import {
  SettingsFieldGrid,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

interface UsageGroupKeyProps {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}

export const UsageGroupKeySection = ({ methods }: UsageGroupKeyProps) => {
  const eventCode = useWatch({ control: methods.control, name: 'eventCode' })

  return (
    <SettingsSection
      title="Usage Grouping"
      description="Optional. Bill each value of an event property (e.g. cluster ID) as a separate invoice line, priced on its own usage."
    >
      <SettingsFieldGrid>
        <PropertyKeyFormField
          name="usageGroupKey"
          control={methods.control}
          eventCode={eventCode ?? ''}
          label="Grouping Key"
          placeholder="cluster_id"
        />
      </SettingsFieldGrid>
    </SettingsSection>
  )
}
