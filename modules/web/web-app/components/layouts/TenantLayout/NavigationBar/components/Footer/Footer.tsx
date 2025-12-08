import { SettingsIcon } from '@md/icons'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { LogOutIcon, TerminalIcon, UserCircle2Icon } from 'lucide-react'
import { Link } from 'react-router-dom'

import Item from '../Items/components/Item/Item'

import type { FunctionComponent, ReactNode } from 'react'

const Footer: FunctionComponent = () => {
  return (
    <footer className="w-full">
      <ul className="max-w-[55px] w-full flex flex-col gap-2">
        <Item to="developers" label="Developer Settings" icon={<TerminalIcon size={20} />} />
        <Item to="settings" label="Settings" icon={<SettingsIcon size={20} />} />
        <FooterAccountDropdown />
      </ul>
    </footer>
  )
}

const UserPreferenceTooltip = ({ children }: { children: ReactNode }) => {
  return (
    <Tooltip delayDuration={0}>
      <TooltipTrigger style={{ width: '100%' }}>{children}</TooltipTrigger>
      <TooltipContent side="right">Account</TooltipContent>
    </Tooltip>
  )
}

export const FooterAccountDropdown: FunctionComponent = () => {
  return (
    <li className="w-full">
      <DropdownMenu>
        <UserPreferenceTooltip>
          <DropdownMenuTrigger asChild>
            <div className="w-[calc(100%-1.5rem)] flex items-center justify-center relative mx-3 py-2 rounded-lg bg-transparent transition-colors duration-200">
              <UserCircle2Icon size={20} className="my-1 cursor-pointer" />
            </div>
          </DropdownMenuTrigger>
        </UserPreferenceTooltip>
        <DropdownMenuContent className="w-56" side="right" align="end" sideOffset={12}>
          <DropdownMenuGroup>
            <Link to="/logout">
              <DropdownMenuItem className="flex gap-2">
                <LogOutIcon size={14} /> Logout
              </DropdownMenuItem>
            </Link>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </li>
  )
}

export default Footer
