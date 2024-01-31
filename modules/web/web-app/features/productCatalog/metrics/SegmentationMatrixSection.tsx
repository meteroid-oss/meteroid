import { FormItem, Input, SelectItem, SidePanel } from '@md/ui'
import { G, O, pipe } from '@mobily/ts-belt'
import { ReactCodeMirrorRef } from '@uiw/react-codemirror'
import { ChangeEventHandler, forwardRef, useEffect, useState } from 'react'
import { Controller, useWatch } from 'react-hook-form'

import { AccordionPanel } from '@/components/AccordionPanel'
import { ControlledSelect } from '@/components/form'
import { JsonEditor } from '@/components/form/JsonEditor'
import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

const linkedValuesPlaceholder = `{
    "AWS": ["eu-west-1", "eu-west-2", "..."],
    "GCP": ["europe-west-1", "europe-west-2", "..."],
    "Azure": ["francecentral", "germanywestcentral", "..."],
  }`
interface BillingMatrixProps {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}
export const SegmentationMatrixSection = ({ methods }: BillingMatrixProps) => {
  const mode = useWatch({ name: 'segmentationMatrix.matrixType', control: methods.control })

  useEffect(() => {
    mode !== 'LINKED' && methods.resetField('segmentationMatrix.linked')
    mode !== 'SINGLE' && methods.resetField('segmentationMatrix.single')
    mode !== 'DOUBLE' && methods.resetField('segmentationMatrix.double')
  }, [mode, methods])

  return (
    <>
      <SidePanel.Separator />
      <SidePanel.Content>
        <AccordionPanel
          title={
            <div className="space-x-4 items-center flex pr-4">
              <h3>Segmentation Matrix</h3>
              <span className="text-xs text-scale-1100">optional</span>
            </div>
          }
          defaultOpen={false}
        >
          <div className="space-y-6">
            <div className="text-sm text-scale-1000 space-y-2">
              <p>
                Specify different pricing based on one or two dimensions. Values are fixed.
                <br />
                For example, you could have different pricing per cloud provider and region, or per
                API endpoint
              </p>
            </div>
            <div>
              <div>
                <div className="space-y-6 ">
                  <ControlledSelect
                    {...methods.withControl('segmentationMatrix.matrixType')}
                    className="max-w-sm"
                  >
                    <SelectItem value="NONE">Unset</SelectItem>
                    <SelectItem value="SINGLE">Single dimension</SelectItem>
                    <SelectItem value="DOUBLE">Two dimensions (independant)</SelectItem>
                    <SelectItem value="LINKED">Two dimensions (dependant)</SelectItem>
                  </ControlledSelect>

                  {mode === 'SINGLE' && (
                    <>
                      <FormItem
                        name="dimension"
                        label="Dimension"
                        {...methods.withError('segmentationMatrix.single.key')}
                      >
                        <Input
                          placeholder="provider"
                          className="max-w-sm"
                          {...methods.register('segmentationMatrix.single.key')}
                        />
                      </FormItem>
                      <FormItem
                        name="values"
                        label="Values"
                        hint="Comma-separated values"
                        {...methods.withError('segmentationMatrix.single.values')}
                      >
                        <Controller
                          render={({ field: { onChange, value, ...rest } }) => {
                            const mappedValue = value?.join(',')
                            const mappedOnChange: ChangeEventHandler<HTMLInputElement> = e =>
                              onChange(e.target.value.split(','))
                            return (
                              <Input
                                placeholder="AWS,Azure Cloud,GCP"
                                onChange={mappedOnChange}
                                value={mappedValue}
                                {...rest}
                              />
                            )
                          }}
                          name="segmentationMatrix.single.values"
                          control={methods.control}
                        />
                      </FormItem>
                    </>
                  )}
                  {mode === 'DOUBLE' && (
                    <>
                      <FormItem
                        name="dimension"
                        label="Dimension"
                        {...methods.withError('segmentationMatrix.double.dimension1.key')}
                      >
                        <Input
                          placeholder="provider"
                          className="max-w-sm"
                          {...methods.register('segmentationMatrix.double.dimension1.key')}
                        />
                      </FormItem>
                      <FormItem
                        label="Values"
                        hint="Comma-separated values"
                        {...methods.withError('segmentationMatrix.double.dimension1.values')}
                      >
                        <Controller
                          render={({ field: { onChange, value, ...rest } }) => {
                            const mappedValue = value?.join(',')
                            const mappedOnChange: ChangeEventHandler<HTMLInputElement> = e =>
                              onChange(e.target.value.split(','))
                            return (
                              <Input
                                placeholder="AWS,Azure Cloud,GCP"
                                onChange={mappedOnChange}
                                value={mappedValue}
                                {...rest}
                              />
                            )
                          }}
                          name="segmentationMatrix.double.dimension1.values"
                          control={methods.control}
                        />
                      </FormItem>
                      <FormItem
                        name="dimension"
                        label="Dimension 2"
                        {...methods.withError('segmentationMatrix.double.dimension2.key')}
                      >
                        <Input
                          placeholder="instance_size"
                          className="max-w-sm"
                          {...methods.register('segmentationMatrix.double.dimension2.key')}
                        />
                      </FormItem>
                      <FormItem
                        name="segmentationMatrix.double.dimension2.values"
                        label="Values"
                        hint="Comma-separated values"
                        {...methods.withError('segmentationMatrix.double.dimension2.values')}
                      >
                        <Controller
                          render={({ field: { onChange, value, ...rest } }) => {
                            const mappedValue = value?.join(',')
                            const mappedOnChange: ChangeEventHandler<HTMLInputElement> = e =>
                              onChange(e.target.value.split(','))
                            return (
                              <Input
                                placeholder="XS,S,M,L,XL"
                                onChange={mappedOnChange}
                                value={mappedValue}
                                {...rest}
                              />
                            )
                          }}
                          name="segmentationMatrix.double.dimension2.values"
                          control={methods.control}
                        />
                      </FormItem>
                    </>
                  )}
                  {mode === 'LINKED' && (
                    <>
                      <FormItem
                        name="dimension"
                        label="Dimension"
                        {...methods.withError('segmentationMatrix.linked.dimensionKey')}
                      >
                        <Input
                          placeholder="provider"
                          className="max-w-sm"
                          {...methods.register('segmentationMatrix.linked.dimensionKey')}
                        />
                      </FormItem>
                      <FormItem
                        name="dimension"
                        label="Relative Dimension"
                        {...methods.withError('segmentationMatrix.linked.linkedDimensionKey')}
                      >
                        <Input
                          className="max-w-sm"
                          placeholder="region"
                          {...methods.register('segmentationMatrix.linked.linkedDimensionKey')}
                        />
                      </FormItem>
                      <FormItem
                        name="values"
                        label="Values"
                        hint="A JSON map of the dimension values"
                        {...methods.withError('segmentationMatrix.linked.values')}
                      >
                        <Controller
                          render={({ field: { onChange, value, ...rest } }) => {
                            return <JsonMapEditor onChange={onChange} value={value} {...rest} />
                          }}
                          name="segmentationMatrix.linked.values"
                          control={methods.control}
                        />
                      </FormItem>
                    </>
                  )}
                </div>
              </div>
            </div>
          </div>
        </AccordionPanel>
      </SidePanel.Content>
    </>
  )
}

const jsonMapSchema = {
  $schema: 'http://json-schema.org/draft-07/schema#',
  type: 'object',
  patternProperties: {
    '^[a-zA-Z0-9_]+$': {
      type: 'array',
      items: { type: 'string', minLength: 1 },
      minItems: 1,
      uniqueItems: true,
    },
  },
  additionalProperties: false,
}

interface JsonMapEditorProps {
  value: [string, ...string[]] | Record<string, [string, ...string[]]>
  onChange: (value: unknown) => void
  onBlur?: () => void
}
const JsonMapEditor = forwardRef<ReactCodeMirrorRef, JsonMapEditorProps>(
  ({ value, onChange, onBlur }: JsonMapEditorProps, ref) => {
    const mappedValue = pipe(
      value,
      O.flatMap(v => O.fromExecution(() => JSON.stringify(v))),
      O.toUndefined
    )
    const [currentValue, setCurrentValue] = useState(mappedValue ?? '')

    useEffect(() => {
      const maybeParsed = O.fromExecution(() => JSON.parse(currentValue))
      const fallback = () =>
        pipe(
          maybeParsed,
          O.match(onChange, () => onChange(currentValue))
        )

      pipe(maybeParsed, O.filter(G.isObject), O.match(onChange, fallback))
    }, [currentValue, onChange])

    return (
      <JsonEditor
        placeholder={linkedValuesPlaceholder}
        onChange={setCurrentValue}
        value={currentValue}
        onBlur={onBlur}
        schema={jsonMapSchema}
        ref={ref}
      />
    )
  }
)
