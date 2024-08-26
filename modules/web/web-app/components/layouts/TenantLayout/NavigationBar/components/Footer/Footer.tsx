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

import { StyledItems as Items } from '../Items/Items.styled'
import Item from '../Items/components/Item/Item'

import { AvatarTrigger, StyledFooter } from './Footer.styled'

import type { FunctionComponent, ReactNode } from 'react'

const Footer: FunctionComponent = () => {
  return (
    <StyledFooter>
      <Items>
        <Item to="developers" label="Developer Settings" icon={<TerminalIcon size={20} />} />
        <Item to="settings" label="Settings" icon={<SettingsIcon size={20} />} />
        <FooterAccountDropdown />
      </Items>
    </StyledFooter>
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
            <AvatarTrigger>
              <UserCircle2Icon size={20} className="my-1 cursor-pointer" />
            </AvatarTrigger>
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
