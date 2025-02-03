import {
  Checkbox,
  FormLabel,
  InputFormField,
  SelectFormField,
  SelectItem,
  FormDescription,
} from '@md/ui'
import { useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'

import { AccordionPanel } from '@/components/AccordionPanel'
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
      <AccordionPanel
        title={
          <div className="space-x-4 items-center flex pr-4">
            <h3>Unit Conversion</h3>
            <span className="text-xs text-muted-foreground">optional</span>
          </div>
        }
        defaultOpen={false}
      >
        <FormDescription className="pb-4">
          Optionaly define a conversion factor for the aggregated value, for example to convert
          Bytes to MB
        </FormDescription>

        <span className="space-x-2 flex">
          <Checkbox
            id="cb-unit-conv"
            checked={enabled}
            onCheckedChange={() => setEnabled(e => !e)}
          />
          <FormLabel htmlFor="cb-unit-conv">Add a conversion</FormLabel>
        </span>
        {enabled && (
          <>
            <InputFormField
              name="aggregation.unitConversion.factor"
              label="Conversion Factor"
              control={methods.control}
              type="number"
              placeholder="1024"
              className="max-w-xs"
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
          </>
        )}
      </AccordionPanel>
    </>
  )
}
