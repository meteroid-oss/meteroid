import { StarGithub } from '@/components/layouts/shared/LayoutHeader/StarGithub'

import HelpPopover from './HelpPopover'
import { TenantDropdown } from './TenantDropdown'
import { ThemeSwitch } from './ThemeSwitch'
import FamilyPicker from '@/components/FamilyPicker'

interface LayoutHeaderProps {
  customHeaderComponents?: React.ReactNode
  headerBorder?: boolean
  familyPicker?: boolean
}

export const LayoutHeader = ({
  customHeaderComponents,
  headerBorder = false,
  familyPicker = false,
}: LayoutHeaderProps) => {
  return (
    <div
      className={`flex  items-center justify-between py-4 pr-5 pl-10 ${
        headerBorder ? 'border-b border-slate-500' : ''
      }`}
    >
      <div className="flex items-center text-sm gap-2">
        <TenantDropdown />
        {familyPicker && <FamilyPicker />}
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
