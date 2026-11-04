import { useQuery } from '@/lib/connectrpc'
import { listCustomPropertyDefinitions } from '@/rpc/api/customproperties/v1/customproperties-CustomPropertiesService_connectquery'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'

import type { ColumnDefinition } from '@/components/CSVImportDialog'

/**
 * CSV-import column describing the tenant's custom properties for `entityType`, listing the
 * accepted `custom_properties.<key>` headers. Empty when the tenant defined none, so the
 * column simply doesn't show up.
 */
export function useCustomPropertyImportColumn(
  entityType: CustomPropertyEntityType
): ColumnDefinition[] {
  const { data } = useQuery(listCustomPropertyDefinitions, {
    entityType,
    includeArchived: false,
  })
  const definitions = data?.definitions ?? []

  if (definitions.length === 0) return []

  return [
    {
      name: 'custom_properties.*',
      tooltipMessage: (
        <div className="space-y-2">
          <p className="font-semibold">Available fields:</p>
          <ul className="list-disc pl-4 space-y-1">
            {definitions.map(def => (
              <li key={def.key}>
                custom_properties.{def.key} ({def.name})
              </li>
            ))}
          </ul>
          <p>
            Including any of these columns makes the file the source of truth: properties you
            don&apos;t list, and empty cells, are cleared. Multi-select values are comma-separated.
          </p>
        </div>
      ),
    },
  ]
}
