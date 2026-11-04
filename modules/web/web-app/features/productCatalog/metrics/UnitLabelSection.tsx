import { InputFormField } from '@md/ui'
import { useEffect, useRef } from 'react'
import { type Control, useFormContext, useWatch } from 'react-hook-form'

import { SettingsFieldGrid, SettingsSection } from '@/features/settings/components/SettingsSection'

interface UnitLabelSectionProps {
  // Create and edit forms differ; both carry `unitSingular` / `unitPlural` at the root.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  control: Control<any>
}

export const UnitLabelSection = ({ control }: UnitLabelSectionProps) => {
  const { trigger } = useFormContext()
  const [singular, plural] = useWatch({ control, name: ['unitSingular', 'unitPlural'] })
  const isFirstRender = useRef(true)

  // "Both or neither" is a cross-field rule: revalidate both so the error moves to the
  // field that is still missing (RHF only re-validates the edited field otherwise).
  useEffect(() => {
    if (isFirstRender.current) {
      isFirstRender.current = false
      return
    }
    void trigger(['unitSingular', 'unitPlural'])
  }, [singular, plural, trigger])

  return (
    <SettingsSection
      title={
        <span className="flex items-center gap-2">
          Unit Label
          <span className="text-xs font-normal text-muted-foreground">optional</span>
        </span>
      }
      description="Used to label quantities and prices, e.g. $12.00 per 1M tokens."
    >
      <SettingsFieldGrid>
        <InputFormField
          name="unitSingular"
          label="Unit (singular)"
          control={control}
          placeholder="token"
          maxLength={32}
        />
        <InputFormField
          name="unitPlural"
          label="Unit (plural)"
          control={control}
          placeholder="tokens"
          maxLength={32}
        />
      </SettingsFieldGrid>
    </SettingsSection>
  )
}
