import { Checkbox, FormLabel, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'

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

  useEffect(() => {
    if (!enabled || aggregationType in ['COUNT', 'COUNT_DISTINCT']) {
      methods.resetField('aggregation.unitConversion')
    }
  }, [enabled, aggregationType, methods])

  if (aggregationType === 'COUNT' || aggregationType === 'COUNT_DISTINCT') {
    return null
  }

  return (
    <>
      <div className="space-y-4">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <h3 className="text-sm font-medium">Unit Conversion</h3>
            <span className="text-xs text-muted-foreground">optional</span>
          </div>
          <p className="text-xs text-muted-foreground">
            Convert aggregated values from smaller to larger units by dividing by the conversion factor.
            For example: 1024 converts bytes to KB, 1048576 converts bytes to MB.
          </p>
        </div>

        <div className="space-y-4 pl-4 border-l-2 border-muted">
          <span className="space-x-2 flex">
            <Checkbox
              id="cb-unit-conv"
              checked={enabled}
              onCheckedChange={() => setEnabled(e => !e)}
            />
            <FormLabel htmlFor="cb-unit-conv">Add a conversion</FormLabel>
          </span>
          {enabled && (
            <div className="space-y-4">
              <InputFormField
                name="aggregation.unitConversion.factor"
                label="Conversion Factor"
                control={methods.control}
                type="number"
                placeholder="1024"
                className="max-w-xs"
                description="Raw usage values will be divided by this factor. Use 1024 for bytes→KB, 1048576 for bytes→MB."
              />

              <SelectFormField
                name="aggregation.unitConversion.rounding"
                label="Rounding"
                control={methods.control}
                className="max-w-xs"
                placeholder="Select a rounding mode"
              >
                <SelectItem value="NONE">None</SelectItem>
                <SelectItem value="UP">Up</SelectItem>
                <SelectItem value="DOWN">Down</SelectItem>
                <SelectItem value="NEAREST">Nearest</SelectItem>
                <SelectItem value="NEAREST_1">Nearest .1</SelectItem>
                <SelectItem value="NEAREST_5">Nearest .5</SelectItem>
              </SelectFormField>
            </div>
          )}
        </div>
      </div>
    </>
  )
}
