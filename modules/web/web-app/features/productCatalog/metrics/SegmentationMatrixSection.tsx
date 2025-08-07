import {
  FormDescription,
  GenericFormField,
  InputFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { G, O, pipe } from '@mobily/ts-belt'
import { ReactCodeMirrorRef } from '@uiw/react-codemirror'
import { forwardRef, useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'

import { AccordionPanel } from '@/components/AccordionPanel'
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
      <AccordionPanel
        title={
          <div className="space-x-4 items-center flex pr-4">
            <h3>Segmentation</h3>
            <span className="text-xs text-muted-foreground">optional</span>
          </div>
        }
        defaultOpen={false}
      >
        <div className="space-y-6">
          <FormDescription>
            <p>
              Specify different pricing based on one or two dimensions with fixed values.
              <br/>
              For example, you could have different pricing for a Compute metric per cloud provider
              and region.
            </p>
          </FormDescription>
          <div>
            <div>
              <div className="space-y-6 ">
                <SelectFormField
                  name="segmentationMatrix.matrixType"
                  control={methods.control}
                  className="max-w-xs"
                >
                  <SelectItem value="NONE">No segmentation</SelectItem>
                  <SelectItem value="SINGLE">Single dimension</SelectItem>
                  <SelectItem value="DOUBLE">Two dimensions (independent)</SelectItem>
                  <SelectItem value="LINKED">Two dimensions (dependent)</SelectItem>
                </SelectFormField>

                {mode === 'SINGLE' && (
                  <>
                    <InputFormField
                      name="segmentationMatrix.single.key"
                      label="Dimension"
                      control={methods.control}
                      placeholder="provider"
                      className="max-w-xs"
                    />

                    <>
                      <InputFormField
                        name="segmentationMatrix.single.values"
                        label="Values"
                        control={methods.control}
                        placeholder="AWS,Azure Cloud,GCP"
                        transformer={{
                          fromInput(value: string) {
                            return value.split(',') as [string, ...string[]]
                          },
                          toInput(value) {
                            return value?.join(',') ?? ''
                          },
                        }}
                      />

                      <FormDescription>Comma-separated values</FormDescription>
                    </>
                  </>
                )}
                {mode === 'DOUBLE' && (
                  <>
                    <InputFormField
                      control={methods.control}
                      name="segmentationMatrix.double.dimension1.key"
                      label="Dimension"
                      placeholder="provider"
                      className="max-w-xs"
                    />
                    <>
                      <InputFormField
                        control={methods.control}
                        name="segmentationMatrix.double.dimension1.values"
                        label="Values"
                        placeholder="AWS,Azure Cloud,GCP"
                        transformer={{
                          fromInput(value: string) {
                            return value.split(',') as [string, ...string[]]
                          },
                          toInput(value) {
                            return value?.join(',') ?? ''
                          },
                        }}
                      />
                      <FormDescription>Comma-separated values</FormDescription>
                    </>

                    <InputFormField
                      control={methods.control}
                      name="segmentationMatrix.double.dimension2.key"
                      label="Dimension 2"
                      placeholder="instance_size"
                      className="max-w-xs"
                    />

                    <>
                      <InputFormField
                        control={methods.control}
                        name="segmentationMatrix.double.dimension2.values"
                        label="Values"
                        placeholder="XS,S,M,L,XL"
                        transformer={{
                          fromInput(value: string) {
                            return value.split(',') as [string, ...string[]]
                          },
                          toInput(value) {
                            return value?.join(',') ?? ''
                          },
                        }}
                      />
                      <FormDescription>Comma-separated values</FormDescription>
                    </>
                  </>
                )}
                {mode === 'LINKED' && (
                  <>
                    <InputFormField
                      control={methods.control}
                      name="segmentationMatrix.linked.dimensionKey"
                      label="Dimension"
                      placeholder="provider"
                      className="max-w-xs"
                    />

                    <InputFormField
                      control={methods.control}
                      name="segmentationMatrix.linked.linkedDimensionKey"
                      label="Relative Dimension"
                      placeholder="region"
                      className="max-w-xs"
                    />

                    <>
                      <GenericFormField
                        control={methods.control}
                        name="segmentationMatrix.linked.values"
                        label="Values"
                        render={({ field }) => {
                          return <JsonMapEditor {...field} />
                        }}
                      />

                      <FormDescription>A JSON map of the dimension values</FormDescription>
                    </>
                  </>
                )}
              </div>
            </div>
          </div>
        </div>
      </AccordionPanel>
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
    const [currentValue, setCurrentValue] = useState(mappedValue)

    useEffect(() => {
      if (currentValue === undefined) return
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
