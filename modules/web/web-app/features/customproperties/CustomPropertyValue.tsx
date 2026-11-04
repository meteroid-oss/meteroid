import {
  CustomPropertyDefinition,
  CustomPropertyType,
} from '@/rpc/api/customproperties/v1/models_pb'

import { formatCustomPropertyValue } from './useCustomPropertyRows'

interface CustomPropertyValueProps {
  def: CustomPropertyDefinition
  value: unknown
}

/** Read-only rendering of a custom-property value, with typed affordances: EMAIL becomes a
 *  `mailto:` link and URL a new-tab link. Everything else falls back to formatted text. */
const isHttpUrl = (s: string): boolean => {
  try {
    return ['http:', 'https:'].includes(new URL(s).protocol)
  } catch {
    return false
  }
}

export const CustomPropertyValue = ({ def, value }: CustomPropertyValueProps) => {
  const text = formatCustomPropertyValue(def, value)
  // Only linkify well-formed values; anything else (e.g. legacy data stored under a former type)
  // falls back to plain text rather than rendering a broken link.
  switch (def.propertyType) {
    case CustomPropertyType.EMAIL:
      return typeof value === 'string' && value.includes('@') ? (
        <a href={`mailto:${value}`} className="text-brand hover:underline">
          {value}
        </a>
      ) : (
        <>{text}</>
      )
    case CustomPropertyType.URL:
      return typeof value === 'string' && isHttpUrl(value) ? (
        <a href={value} target="_blank" rel="noreferrer" className="text-brand hover:underline">
          {value}
        </a>
      ) : (
        <>{text}</>
      )
    default:
      return <>{text}</>
  }
}
