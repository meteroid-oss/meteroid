import { Button, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { AlertTriangleIcon } from 'lucide-react'
import { ReactNode } from 'react'

import {
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

export type DangerAction = {
  title: string
  description: ReactNode
  buttonLabel: string
  onConfirm: () => void
  // Shown in the confirmation dialog before onConfirm runs. confirmLabel
  // defaults to buttonLabel.
  confirmation: {
    header: string
    message: string | JSX.Element
    confirmLabel?: string
  }
  // null/undefined = enabled. When set, the button is disabled and the reason
  // is shown in a tooltip.
  disabledReason?: string | null
}

// Shared "Danger zone" block for settings tabs: a single destructive-framed
// group with one divided row per action. Red is carried by the frame and the
// action buttons; row titles stay neutral to keep the zone calm. Each action
// is gated behind a danger-styled confirmation dialog.
export const DangerZone = ({ actions }: { actions: DangerAction[] }) => {
  const confirm = useConfirmationModal()

  return (
    <SettingsSection
      title={
        <span className="flex items-center gap-2">
          <AlertTriangleIcon size={15} className="text-destructive" />
          Danger zone
        </span>
      }
    >
      <SettingsRowGroup className="border-destructive/40 bg-destructive/[0.035]">
        {actions.map((action, i) => (
          <SettingsRow key={i} title={action.title} description={action.description}>
            <DangerButton
              buttonLabel={action.buttonLabel}
              disabledReason={action.disabledReason}
              onClick={() =>
                confirm(
                  action.onConfirm,
                  { header: action.confirmation.header, message: action.confirmation.message },
                  {
                    buttonLabel: action.confirmation.confirmLabel ?? action.buttonLabel,
                    danger: true,
                  }
                )
              }
            />
          </SettingsRow>
        ))}
      </SettingsRowGroup>
    </SettingsSection>
  )
}

const DangerButton = ({
  buttonLabel,
  onClick,
  disabledReason,
}: {
  buttonLabel: string
  onClick: () => void
  disabledReason?: string | null
}) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <span>
        <Button
          variant="destructive"
          size="sm"
          onClick={onClick}
          disabled={disabledReason != null}
        >
          {buttonLabel}
        </Button>
      </span>
    </TooltipTrigger>
    {disabledReason != null && <TooltipContent>{disabledReason}</TooltipContent>}
  </Tooltip>
)
