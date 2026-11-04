import { useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { listCustomPropertyDefinitions } from '@/rpc/api/customproperties/v1/customproperties-CustomPropertiesService_connectquery'
import {
  CustomPropertyEntityType,
  CustomPropertyType,
} from '@/rpc/api/customproperties/v1/models_pb'

import {
  FieldWidget,
  FilterOp,
  NATIVE_FIELDS,
  NativeField,
  NativeFieldEntity,
  allowedOpsForPropertyType,
} from './filterTypes'

/** A field the user can filter on, resolved from native columns + custom-property definitions. */
export interface ResolvedField {
  /** Wire field: native column id, or `cp:<key>` for a custom property. */
  field: string
  label: string
  widget: FieldWidget
  ops: FilterOp[]
  options?: { value: string; label: string }[]
  /** The picker heading: 'Properties' / 'Custom' for entity fields, or a page's own group. */
  group: string
}

function widgetForType(type: CustomPropertyType): FieldWidget {
  switch (type) {
    case CustomPropertyType.NUMBER:
      return 'number'
    case CustomPropertyType.BOOLEAN:
      return 'boolean'
    case CustomPropertyType.DATE:
      return 'date'
    case CustomPropertyType.DATETIME:
      return 'datetime'
    case CustomPropertyType.SINGLE_SELECT:
    case CustomPropertyType.MULTI_SELECT:
      return 'select'
    default:
      return 'text'
  }
}

const ENTITY_KEY: Record<number, NativeFieldEntity> = {
  [CustomPropertyEntityType.CUSTOMER]: 'customer',
  [CustomPropertyEntityType.SUBSCRIPTION]: 'subscription',
  [CustomPropertyEntityType.INVOICE]: 'invoice',
  [CustomPropertyEntityType.CREDIT_NOTE]: 'credit_note',
  [CustomPropertyEntityType.QUOTE]: 'quote',
}

/**
 * Filterable fields for an entity list: native columns (from {@link NATIVE_FIELDS}) plus one entry
 * per active custom-property definition. Reuses the same definitions query the columns use, so the
 * filter picker and the opt-in columns stay in lockstep.
 */
export function useFilterFields(entityType: CustomPropertyEntityType): ResolvedField[] {
  return useFilterFieldsQuery(entityType).fields
}

/** {@link useFilterFields} plus whether the custom-property definitions have loaded. */
export function useFilterFieldsQuery(entityType: CustomPropertyEntityType): {
  fields: ResolvedField[]
  isLoaded: boolean
} {
  const { data, isSuccess, isError } = useQuery(listCustomPropertyDefinitions, {
    entityType,
    includeArchived: false,
  })

  const fields = useMemo(() => {
    const definitions = data?.definitions ?? []
    const natives: NativeField[] = NATIVE_FIELDS[ENTITY_KEY[entityType]] ?? []
    const nativeFields: ResolvedField[] = natives.map(f => ({
      field: f.id,
      label: f.label,
      widget: f.widget,
      ops: f.ops,
      options: f.options,
      group: 'Properties',
    }))

    const customFields: ResolvedField[] = definitions.map(def => ({
      field: `cp:${def.key}`,
      label: def.name,
      widget: widgetForType(def.propertyType),
      ops: allowedOpsForPropertyType(def.propertyType),
      options: def.config?.options?.map(o => ({ value: o.value, label: o.label || o.value })),
      group: 'Custom',
    }))

    return [...nativeFields, ...customFields]
  }, [entityType, data])

  // On error, fall back to native fields only rather than blocking the list forever.
  return { fields, isLoaded: isSuccess || isError }
}
