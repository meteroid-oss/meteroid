import {
  Button,
  FormDescription,
  GenericFormField,
  Input,
  Label,
  SelectFormField,
  SelectItem,
  Separator,
} from '@md/ui'
import { forwardRef, useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'

import { ValuesTagInput } from '@/components/form'
import { PropertyKeyFormField } from '@/features/productCatalog/metrics/PropertyKeyFormField'
import { PropertyValueTags } from '@/features/productCatalog/metrics/PropertyValueField'
import {
  SettingsFieldGrid,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { Methods } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

interface BillingMatrixProps {
  methods: Methods<schemas.billableMetrics.CreateBillableMetricSchema>
}

export const SegmentationMatrixSection = ({ methods }: BillingMatrixProps) => {
  const mode = useWatch({ name: 'segmentationMatrix.matrixType', control: methods.control })
  const eventCode = useWatch({ name: 'eventCode', control: methods.control }) ?? ''
  // Each dimension's values autocomplete against that dimension's own property key.
  const singleKey = useWatch({ name: 'segmentationMatrix.single.key', control: methods.control }) ?? ''
  const dim1Key =
    useWatch({ name: 'segmentationMatrix.double.dimension1.key', control: methods.control }) ?? ''
  const dim2Key =
    useWatch({ name: 'segmentationMatrix.double.dimension2.key', control: methods.control }) ?? ''

  const { resetField } = methods
  useEffect(() => {
    mode !== 'LINKED' && resetField('segmentationMatrix.linked')
    mode !== 'SINGLE' && resetField('segmentationMatrix.single')
    mode !== 'DOUBLE' && resetField('segmentationMatrix.double')
  }, [mode, resetField])

  return (
    <SettingsSection
      title="Segmentation"
      description="Optional. Apply different pricing based on predefined categories (e.g. cloud provider or region)."
    >
      <div className="space-y-4">
        <SettingsFieldGrid>
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
        </SettingsFieldGrid>

        {mode === 'SINGLE' && (
          <div className="space-y-4">
            <SettingsFieldGrid>
              <PropertyKeyFormField
                name="segmentationMatrix.single.key"
                label="Dimension Name"
                control={methods.control}
                eventCode={eventCode}
                placeholder="provider"
              />
              <GenericFormField
                name="segmentationMatrix.single.values"
                label="Values"
                control={methods.control}
                render={({ field }) => (
                  <PropertyValueTags
                    eventCode={eventCode}
                    propertyKey={singleKey}
                    value={field.value}
                    onChange={field.onChange}
                    placeholder="AWS, Azure, GCP"
                  />
                )}
              />
            </SettingsFieldGrid>
            <FormDescription>
              Each value will create a separate pricing tier. Press Enter or comma to add values.
            </FormDescription>
          </div>
        )}

        {mode === 'DOUBLE' && (
          <div className="space-y-4">
            <div className="space-y-4">
              <div className="space-y-3">
                <p className="text-xs font-medium text-muted-foreground">First Dimension</p>
                <SettingsFieldGrid>
                  <PropertyKeyFormField
                    control={methods.control}
                    name="segmentationMatrix.double.dimension1.key"
                    label="Name"
                    eventCode={eventCode}
                    placeholder="provider"
                  />
                  <GenericFormField
                    control={methods.control}
                    name="segmentationMatrix.double.dimension1.values"
                    label="Values"
                    render={({ field }) => (
                      <PropertyValueTags
                        eventCode={eventCode}
                        propertyKey={dim1Key}
                        value={field.value}
                        onChange={field.onChange}
                        placeholder="AWS, Azure, GCP"
                      />
                    )}
                  />
                </SettingsFieldGrid>
              </div>

              <Separator />

              <div className="space-y-3">
                <p className="text-xs font-medium text-muted-foreground">Second Dimension</p>
                <SettingsFieldGrid>
                  <PropertyKeyFormField
                    control={methods.control}
                    name="segmentationMatrix.double.dimension2.key"
                    label="Name"
                    eventCode={eventCode}
                    placeholder="instance_size"
                  />
                  <GenericFormField
                    control={methods.control}
                    name="segmentationMatrix.double.dimension2.values"
                    label="Values"
                    render={({ field }) => (
                      <PropertyValueTags
                        eventCode={eventCode}
                        propertyKey={dim2Key}
                        value={field.value}
                        onChange={field.onChange}
                        placeholder="XS, S, M, L, XL"
                      />
                    )}
                  />
                </SettingsFieldGrid>
              </div>
            </div>
            <FormDescription>
              This creates a pricing matrix. Each combination will have its own pricing tier.
            </FormDescription>
          </div>
        )}

        {mode === 'LINKED' && (
          <div className="space-y-4">
            <SettingsFieldGrid>
              <PropertyKeyFormField
                control={methods.control}
                name="segmentationMatrix.linked.dimensionKey"
                label="Primary Dimension"
                eventCode={eventCode}
                placeholder="provider"
              />
              <PropertyKeyFormField
                control={methods.control}
                name="segmentationMatrix.linked.linkedDimensionKey"
                label="Dependent Dimension"
                eventCode={eventCode}
                placeholder="region"
              />
            </SettingsFieldGrid>

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
    </SettingsSection>
  )
}

interface JsonMapEditorProps {
  value: [string, ...string[]] | Record<string, string[]> | undefined
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
