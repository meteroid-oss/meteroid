import {
  Button,
  FormDescription,
  GenericFormField,
  Input,
  InputFormField,
  Label,
  SelectFormField,
  SelectItem,
  Separator,
} from '@md/ui'
import { forwardRef, useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'

import { AccordionPanel } from '@/components/AccordionPanel'
import { ValuesTagInput } from '@/features/productCatalog/metrics/ValuesTagInput'
import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

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
              Apply different pricing based on predefined categories with fixed values.
              <br />
              For example, different pricing for a Compute metric per cloud provider (AWS, GCP,
              Azure) and region (us-east-1, eu-west-1).
            </p>
          </FormDescription>

          <div className="max-w-md">
            <SelectFormField
              name="segmentationMatrix.matrixType"
              control={methods.control}
              label="Segmentation Type"
            >
              <SelectItem value="NONE">
                <div>
                  <div className="font-medium">No Segmentation</div>
                  <div className="text-xs text-muted-foreground">
                    Single flat pricing for all usage
                  </div>
                </div>
              </SelectItem>
              <SelectItem value="SINGLE">
                <div>
                  <div className="font-medium">Single Dimension</div>
                  <div className="text-xs text-muted-foreground">
                    Different pricing per category (e.g. AWS, GCP, Azure)
                  </div>
                </div>
              </SelectItem>
              <SelectItem value="DOUBLE">
                <div>
                  <div className="font-medium">Two Dimensions</div>
                  <div className="text-xs text-muted-foreground">
                    Pricing matrix with two categories (e.g. provider × region)
                  </div>
                </div>
              </SelectItem>
              <SelectItem value="LINKED">
                <div>
                  <div className="font-medium">Dependent Dimensions</div>
                  <div className="text-xs text-muted-foreground">
                    Linked categories with dependencies (e.g. AWS has specific regions)
                  </div>
                </div>
              </SelectItem>
            </SelectFormField>
          </div>

          {mode === 'SINGLE' && (
            <div className="space-y-4">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <InputFormField
                  name="segmentationMatrix.single.key"
                  label="Dimension Name"
                  control={methods.control}
                  placeholder="provider"
                />
                <GenericFormField
                  name="segmentationMatrix.single.values"
                  label="Values"
                  control={methods.control}
                  render={({ field }) => (
                    <ValuesTagInput
                      value={field.value}
                      onChange={field.onChange}
                      placeholder="AWS, Azure, GCP"
                    />
                  )}
                />
              </div>
              <FormDescription>
                Each value will create a separate pricing tier. Press Enter or comma to add values.
              </FormDescription>
            </div>
          )}

          {mode === 'DOUBLE' && (
            <div className="space-y-6">
              <div className="space-y-4">
                <div className="space-y-3">
                  <p className="text-sm font-medium text-muted-foreground">First Dimension</p>
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <InputFormField
                      control={methods.control}
                      name="segmentationMatrix.double.dimension1.key"
                      label="Name"
                      placeholder="provider"
                    />
                    <GenericFormField
                      control={methods.control}
                      name="segmentationMatrix.double.dimension1.values"
                      label="Values"
                      render={({ field }) => (
                        <ValuesTagInput
                          value={field.value}
                          onChange={field.onChange}
                          placeholder="AWS, Azure, GCP"
                        />
                      )}
                    />
                  </div>
                </div>

                <Separator />

                <div className="space-y-3">
                  <p className="text-sm font-medium text-muted-foreground">Second Dimension</p>
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <InputFormField
                      control={methods.control}
                      name="segmentationMatrix.double.dimension2.key"
                      label="Name"
                      placeholder="instance_size"
                    />
                    <GenericFormField
                      control={methods.control}
                      name="segmentationMatrix.double.dimension2.values"
                      label="Values"
                      render={({ field }) => (
                        <ValuesTagInput
                          value={field.value}
                          onChange={field.onChange}
                          placeholder="XS, S, M, L, XL"
                        />
                      )}
                    />
                  </div>
                </div>
              </div>
              <FormDescription>
                This creates a pricing matrix. Each combination will have its own pricing tier.
              </FormDescription>
            </div>
          )}

          {mode === 'LINKED' && (
            <div className="space-y-4">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <InputFormField
                  control={methods.control}
                  name="segmentationMatrix.linked.dimensionKey"
                  label="Primary Dimension"
                  placeholder="provider"
                />
                <InputFormField
                  control={methods.control}
                  name="segmentationMatrix.linked.linkedDimensionKey"
                  label="Dependent Dimension"
                  placeholder="region"
                />
              </div>

              <div className="space-y-2">
                <GenericFormField
                  control={methods.control}
                  name="segmentationMatrix.linked.values"
                  label="Dimension Mapping"
                  render={({ field }) => {
                    return <LinkedDimensionsEditor {...field} />
                  }}
                />
                <FormDescription>
                  Define which dependent values are available for each primary value.
                </FormDescription>
              </div>
            </div>
          )}
        </div>
      </AccordionPanel>
    </>
  )
}

interface JsonMapEditorProps {
  value: [string, ...string[]] | Record<string, [string, ...string[]]> | undefined
  onChange: (value: unknown) => void
}

const LinkedDimensionsEditor = forwardRef<HTMLDivElement, JsonMapEditorProps>(
  ({ value, onChange }, _ref) => {
    const mappedData = typeof value === 'object' && value !== null ? value : {}
    const [entries, setEntries] = useState<Array<{ key: string; values: string[] }>>(() => {
      return Object.entries(mappedData).map(([key, vals]) => ({
        key,
        values: Array.isArray(vals) ? vals : [],
      }))
    })

    useEffect(() => {
      const newMappedData: Record<string, [string, ...string[]]> = {}
      entries.forEach(({ key, values }) => {
        const trimmedKey = key.trim()
        if (trimmedKey && values.length > 0) {
          newMappedData[trimmedKey] = values as [string, ...string[]]
        }
      })
      onChange(newMappedData)
    }, [entries, onChange])

    const addEntry = () => {
      setEntries(prev => [...prev, { key: '', values: [] }])
    }

    const removeEntry = (index: number) => {
      setEntries(prev => prev.filter((_, i) => i !== index))
    }

    const updateKey = (index: number, key: string) => {
      setEntries(prev => prev.map((entry, i) => (i === index ? { ...entry, key } : entry)))
    }

    const updateValues = (index: number, values: string[]) => {
      setEntries(prev => prev.map((entry, i) => (i === index ? { ...entry, values } : entry)))
    }

    // Detect JSON pasting in the first key field (hidden feature for power users)
    const handleFirstKeyChange = (value: string) => {
      if (value.trim().startsWith('{') && value.includes(':')) {
        try {
          const parsed = JSON.parse(value)
          if (typeof parsed === 'object' && parsed !== null) {
            const newEntries = Object.entries(parsed).map(([key, vals]) => ({
              key,
              values: Array.isArray(vals) ? vals.map(String) : [String(vals)],
            }))
            setEntries(newEntries)
            return
          }
        } catch {
          // Not valid JSON, continue with regular handling
        }
      }
      updateKey(0, value)
    }

    return (
      <div className="space-y-3">
        {entries.length === 0 && (
          <div className="text-sm text-muted-foreground p-4 border border-dashed rounded-lg text-center">
            No mappings defined. Add one below.
          </div>
        )}

        {entries.length > 0 && (
          <div className="flex gap-2 items-center text-xs text-muted-foreground">
            <Label className="flex-1">Primary value</Label>
            <Label className="flex-[2]">Linked values</Label>
            <div className="w-[72px]" />
          </div>
        )}

        {entries.map((entry, index) => (
          <div key={index} className="flex gap-2 items-start">
            <div className="flex-1">
              <Input
                value={entry.key}
                onChange={e =>
                  index === 0
                    ? handleFirstKeyChange(e.target.value)
                    : updateKey(index, e.target.value)
                }
                placeholder="Provider (e.g. AWS)"
              />
            </div>
            <div className="flex-[2]">
              <ValuesTagInput
                value={entry.values}
                onChange={vals => updateValues(index, vals)}
                placeholder="us-east-1, us-west-2"
              />
            </div>
            <Button type="button" variant="outline" size="sm" onClick={() => removeEntry(index)}>
              Remove
            </Button>
          </div>
        ))}
        <Button type="button" variant="outline" size="sm" onClick={addEntry} className="w-full">
          Add Mapping
        </Button>
        <FormDescription className="text-xs">
          Example: AWS → us-east-1, us-west-2 | GCP → europe-west1, asia-east1
        </FormDescription>
      </div>
    )
  }
)
