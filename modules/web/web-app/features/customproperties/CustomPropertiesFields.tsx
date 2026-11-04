/**
 * CustomPropertiesFields — renders a form section driven by the tenant's custom property
 * definitions for a given entity type. Each definition becomes a typed input bound to
 * `customProperties.<key>` on the surrounding react-hook-form.
 */
import {
  Badge,
  cn,
  GenericFormField,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  Textarea,
} from '@md/ui'
import { Control, FieldPath, FieldValues } from 'react-hook-form'

import { useQuery } from '@/lib/connectrpc'
import { listCustomPropertyDefinitions } from '@/rpc/api/customproperties/v1/customproperties-CustomPropertiesService_connectquery'
import {
  CustomPropertyDefinition,
  CustomPropertyEntityType,
  CustomPropertyType,
} from '@/rpc/api/customproperties/v1/models_pb'

export interface CustomPropertiesFieldsProps<T extends FieldValues> {
  control: Control<T>
  entityType: CustomPropertyEntityType
  /** Section heading. Pass `null` to render the fields without a heading. */
  title?: string | null
  /** Override the heading classes to match the surrounding form. */
  titleClassName?: string
  /** Field layout, matched to the host form (defaults to vertical). */
  layout?: 'vertical' | 'horizontal'
}

export const CustomPropertiesFields = <T extends FieldValues>({
  control,
  entityType,
  title = 'Custom properties',
  titleClassName,
  layout = 'vertical',
}: CustomPropertiesFieldsProps<T>) => {
  const { data } = useQuery(listCustomPropertyDefinitions, { entityType, includeArchived: false })
  const definitions = data?.definitions ?? []

  if (definitions.length === 0) return null

  return (
    <div className="space-y-3">
      {title !== null && (
        <div className={cn('text-[15px] font-medium', titleClassName)}>{title}</div>
      )}
      {definitions.map(def => (
        <GenericFormField
          key={def.id}
          control={control}
          name={`customProperties.${def.key}` as FieldPath<T>}
          label={def.name}
          description={def.description || undefined}
          layout={layout}
          render={({ field, className }) => (
            <PropertyInput
              def={def}
              value={field.value}
              onChange={field.onChange}
              className={className}
            />
          )}
        />
      ))}
    </div>
  )
}

const pad = (n: number) => String(n).padStart(2, '0')

/** RFC3339 UTC (as stored) → the local `YYYY-MM-DDTHH:MM` a `datetime-local` input expects. */
const toDatetimeLocalInput = (v: unknown): string => {
  if (typeof v !== 'string' || !v) return ''
  const d = new Date(v)
  if (isNaN(d.getTime())) return ''
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}

/** A `datetime-local` value (local wall-clock) → RFC3339 UTC for the backend to normalize. */
const fromDatetimeLocalInput = (local: string): string | undefined => {
  if (!local) return undefined
  const d = new Date(local)
  return isNaN(d.getTime()) ? undefined : d.toISOString()
}

interface PropertyInputProps {
  def: CustomPropertyDefinition
  value: unknown
  onChange: (value: unknown) => void
  className?: string
}

const PropertyInput = ({ def, value, onChange, className }: PropertyInputProps) => {
  const options = def.config?.options ?? []

  switch (def.propertyType) {
    case CustomPropertyType.NUMBER:
      return (
        <Input
          type="number"
          className={className}
          value={value === undefined || value === null ? '' : String(value)}
          onChange={e => onChange(e.target.value === '' ? undefined : Number(e.target.value))}
        />
      )
    case CustomPropertyType.BOOLEAN:
      return <Switch checked={Boolean(value)} onCheckedChange={onChange} />
    case CustomPropertyType.DATE:
      return (
        <Input
          type="date"
          className={className}
          value={(value as string) ?? ''}
          onChange={e => onChange(e.target.value || undefined)}
        />
      )
    case CustomPropertyType.DATETIME:
      return (
        <Input
          type="datetime-local"
          className={className}
          value={toDatetimeLocalInput(value)}
          onChange={e => onChange(fromDatetimeLocalInput(e.target.value))}
        />
      )
    case CustomPropertyType.EMAIL:
      return (
        <Input
          type="email"
          inputMode="email"
          className={className}
          value={(value as string) ?? ''}
          onChange={e => onChange(e.target.value || undefined)}
        />
      )
    case CustomPropertyType.URL:
      return (
        <Input
          type="url"
          inputMode="url"
          className={className}
          value={(value as string) ?? ''}
          onChange={e => onChange(e.target.value || undefined)}
        />
      )
    case CustomPropertyType.SINGLE_SELECT:
      return (
        <Select value={(value as string) ?? ''} onValueChange={onChange}>
          <SelectTrigger className={className}>
            <SelectValue placeholder="Select…" />
          </SelectTrigger>
          <SelectContent>
            {options.map(o => (
              <SelectItem key={o.value} value={o.value}>
                {o.label || o.value}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )
    case CustomPropertyType.MULTI_SELECT: {
      const selected = Array.isArray(value) ? (value as string[]) : []
      const toggle = (v: string) =>
        onChange(selected.includes(v) ? selected.filter(x => x !== v) : [...selected, v])
      return (
        <div className={cn('flex flex-wrap gap-2', className)}>
          {options.map(o => (
            <Badge
              key={o.value}
              variant={selected.includes(o.value) ? 'default' : 'outline'}
              className="cursor-pointer"
              onClick={() => toggle(o.value)}
            >
              {o.label || o.value}
            </Badge>
          ))}
        </div>
      )
    }
    case CustomPropertyType.JSON:
      return (
        <Textarea
          rows={3}
          className={className}
          value={(value as string) ?? ''}
          onChange={e => onChange(e.target.value || undefined)}
        />
      )
    default:
      // TEXT
      return (
        <Input
          className={className}
          value={(value as string) ?? ''}
          onChange={e => onChange(e.target.value || undefined)}
        />
      )
  }
}

/** Serializes a `customProperties` form record to the JSON string carried by the entity proto. */
export function serializeCustomProperties(
  values?: Record<string, unknown>
): string | undefined {
  if (!values) return undefined
  const cleaned: Record<string, unknown> = {}
  for (const [key, v] of Object.entries(values)) {
    if (v === undefined || v === null || v === '') continue
    if (Array.isArray(v) && v.length === 0) continue
    cleaned[key] = v
  }
  return Object.keys(cleaned).length > 0 ? JSON.stringify(cleaned) : undefined
}

/** Parses the JSON-encoded `customProperties` returned by the entity proto into form values. */
export function parseCustomProperties(json?: string): Record<string, unknown> {
  if (!json) return {}
  try {
    const parsed = JSON.parse(json)
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : {}
  } catch {
    return {}
  }
}
