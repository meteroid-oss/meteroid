import { Checkbox, FormLabel, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'

import {
  SettingsFieldGrid,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

interface Props {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}

export const UnitConversionSection = ({ methods }: Props) => {
  const [enabled, setEnabled] = useState(false)
  const aggregationType = useWatch({
    control: methods.control,
    name: 'aggregation.aggregationType',
  })

  const { resetField } = methods
  useEffect(() => {
    if (!enabled || ['COUNT', 'COUNT_DISTINCT'].includes(aggregationType)) {
      resetField('aggregation.unitConversion')
    }
  }, [enabled, aggregationType, resetField])

  if (aggregationType === 'COUNT' || aggregationType === 'COUNT_DISTINCT') {
    return null
  }

  return (
    <SettingsSection
      title={
        <span className="flex items-center gap-2">
          Unit Conversion
          <span className="text-xs font-normal text-muted-foreground">optional</span>
        </span>
      }
      description="Convert aggregated values from smaller to larger units by dividing by the conversion factor. For example: 1024 converts bytes to KB, 1048576 converts bytes to MB."
    >
      <div className="space-y-4">
        <span className="space-x-2 flex">
          <Checkbox
            id="cb-unit-conv"
            checked={enabled}
            onCheckedChange={() => setEnabled(e => !e)}
          />
          <FormLabel htmlFor="cb-unit-conv">Add a conversion</FormLabel>
        </span>
        {enabled && (
          <SettingsFieldGrid>
            <InputFormField
              name="aggregation.unitConversion.factor"
              label="Conversion Factor"
              control={methods.control}
              type="number"
              placeholder="1024"
              description="Raw usage values will be divided by this factor. Use 1024 for bytes→KB, 1048576 for bytes→MB."
            />

            <SelectFormField
              name="aggregation.unitConversion.rounding"
              label="Rounding"
              control={methods.control}
              placeholder="Select a rounding mode"
            >
              <SelectItem value="NONE">None</SelectItem>
              <SelectItem value="UP">Up</SelectItem>
              <SelectItem value="DOWN">Down</SelectItem>
              <SelectItem value="NEAREST">Nearest</SelectItem>
              {/*<SelectItem value="NEAREST_1">Nearest .1</SelectItem>*/}
              {/*<SelectItem value="NEAREST_5">Nearest .5</SelectItem>*/}
            </SelectFormField>
          </SettingsFieldGrid>
        )}
      </div>
    </SettingsSection>
  )
}
