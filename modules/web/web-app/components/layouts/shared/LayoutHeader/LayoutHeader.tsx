import { StarGithub } from '@/components/layouts/shared/LayoutHeader/StarGithub'

import HelpPopover from './HelpPopover'
import { TenantDropdown } from './TenantDropdown'
import { ThemeSwitch } from './ThemeSwitch'

interface LayoutHeaderProps {
  customHeaderComponents?: React.ReactNode
  headerBorder?: boolean
}

export const LayoutHeader = ({
  customHeaderComponents,
  headerBorder = false,
}: LayoutHeaderProps) => {
  return (
    <div
      className={`flex  items-center justify-between py-4 pr-5 pl-10 ${
        headerBorder ? 'border-b border-scale-500' : ''
      }`}
    >
      <div className="flex items-center text-sm">
        <TenantDropdown />
      </div>
      <div className="flex items-center space-x-2">
        {customHeaderComponents && customHeaderComponents}
        <HelpPopover />
        <StarGithub />
        <ThemeSwitch />
      </div>
    </div>
  )
}
