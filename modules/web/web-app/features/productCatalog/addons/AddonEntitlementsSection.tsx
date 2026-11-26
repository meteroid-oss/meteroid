import { EntitlementRowsEditor } from '@/features/entitlements/rows/EntitlementRowsEditor'
import { usePermissions } from '@/hooks/usePermissions'

export const AddonEntitlementsSection = ({
  addonId,
  maxInstances,
  seats,
}: {
  addonId: string
  maxInstances?: number
  /** A seat add-on: held once, its rows multiply by the seats held. */
  seats?: boolean
}) => {
  const canEdit = usePermissions().has('catalog:manage')
  return (
    <EntitlementRowsEditor
      owner={{ case: 'addOnId', value: addonId }}
      multiInstance={seats || maxInstances !== 1}
      canEdit={canEdit}
      title="Entitlements"
    />
  )
}
