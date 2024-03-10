import { Checkbox, CheckboxFormItem, FormItem, Input, SelectItem, SidePanel } from '@md/ui'
import { useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'

import { ControlledSelect } from '@/components/form'
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
      <SidePanel.Separator />
      <SidePanel.Content>
        <div className="space-y-6 py-6">
          <div className="space-y-2">
            <div className="space-x-4 items-center flex pr-4">
              <h3>Unit Conversion</h3>
              <span className="text-xs text-muted-foreground">optional</span>
            </div>
            <p className="text-sm text-slate-1000">
              Optionaly define a conversion factor for the aggregated value, for example to convert
              Bytes to MB
            </p>
          </div>
          <CheckboxFormItem label="Add a conversion" name="cb-unit-conv">
            <Checkbox
              id="cb-unit-conv"
              checked={enabled}
              onCheckedChange={() => setEnabled(e => !e)}
            />
          </CheckboxFormItem>
          {enabled && (
            <>
              <FormItem
                name="conversionFactor"
                label="Conversion Factor"
                error={methods.formState.errors.aggregation?.unitConversion?.factor?.message}
              >
                <Input
                  type="number"
                  placeholder="1024"
                  {...methods.register('aggregation.unitConversion.factor')}
                  className="max-w-sm"
                />
              </FormItem>
              <FormItem
                name="conversionFactor"
                label="Rounding"
                error={methods.formState.errors.aggregation?.unitConversion?.rounding?.message}
              >
                <ControlledSelect
                  {...methods.withControl('aggregation.unitConversion.rounding')}
                  className="max-w-sm"
                  placeholder="Select a rounding mode"
                >
                  <SelectItem value="NONE">None</SelectItem>
                  <SelectItem value="UP">Up</SelectItem>
                  <SelectItem value="DOWN">Down</SelectItem>
                  <SelectItem value="NEAREST">Nearest</SelectItem>
                  <SelectItem value="NEAREST_1">Nearest .1</SelectItem>
                  <SelectItem value="NEAREST_5">Nearest .5</SelectItem>
                </ControlledSelect>
              </FormItem>
            </>
          )}
        </div>
      </SidePanel.Content>
    </>
  )
}
