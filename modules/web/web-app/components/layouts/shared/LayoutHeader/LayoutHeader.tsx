import { StarGithub } from '@/components/layouts/shared/LayoutHeader/StarGithub'

import HelpPopover from './HelpPopover'
import { TenantDropdown } from './TenantDropdown'
import { ThemeSwitch } from './ThemeSwitch'

interface LayoutHeaderProps {
  customHeaderComponents?: React.ReactNode
  headerBorder?: boolean
  title?: string
}

export const LayoutHeader = ({
  customHeaderComponents,
  headerBorder = false,
  title,
}: LayoutHeaderProps) => {
  return (
    <div
      className={`flex  items-center justify-between py-4 ${
        headerBorder ? 'border-b border-border' : ''
      }`}
    >
      <div className="flex items-center text-sm gap-2">
        <TenantDropdown />
        {title && <h3 className="font-semibold pl-2 text-base">{title}</h3>}
      </div>
      <div className="flex items-center space-x-1">
        {customHeaderComponents && customHeaderComponents}
        <HelpPopover />
        <StarGithub />
        <ThemeSwitch />
      </div>
    </div>
  )
}
