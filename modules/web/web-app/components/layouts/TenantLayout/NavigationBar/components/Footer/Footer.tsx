import { SettingsIcon } from '@md/icons'
import {
  TooltipTrigger,
  Tooltip,
  TooltipContent,
  DropdownMenuGroup,
  DropdownMenuItem,

  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuSeparator,
  DropdownMenuTrigger} from '@md/ui'
import {
  LogOutIcon,
  TerminalIcon,
  UserCircle2Icon,
} from 'lucide-react'
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
        <Item to="settings" label="Tenant Settings" icon={<SettingsIcon size={20} />} />
        <FooterAccountDropdown />
      </Items>
    </StyledFooter>
  )
}

const UserPreferenceTooltip = ({ children }: { children: ReactNode }) => {
  return (
    <Tooltip delayDuration={0}>
      <TooltipTrigger style={{ width: '100%' }}>{children}</TooltipTrigger>
      <TooltipContent side="right">Account Settings</TooltipContent>
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
            <Link to="/account/me">
              <DropdownMenuItem className="flex gap-2">
                <SettingsIcon size={14} /> Account Preferences
              </DropdownMenuItem>
            </Link>

            <DropdownMenuSeparator />

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

  // return (
  //   <Dropdown
  //     side="right"
  //     align="start"
  //     className="mb-2"
  //     overlay={
  //       <div className="pl-4">
  //         <Link to="/account/me">
  //           <Dropdown.Item
  //             key="header"
  //             icon={<SettingsIcon size={14} />}
  //             className="hover:bg-slate-500"
  //           >
  //             Account Preferences
  //           </Dropdown.Item>
  //         </Link>
  //         <Dropdown.Separator />
  //         <Dropdown.Label>Theme</Dropdown.Label>
  //         <Dropdown.RadioGroup
  //           key="theme"
  //           value={isDarkMode ? 'dark' : 'light'}
  //           onChange={e => setDarkMode(e === 'dark')}
  //         >
  //           <Dropdown.Radio value="dark" className="hover:bg-slate-500">
  //             Dark
  //           </Dropdown.Radio>
  //           <Dropdown.Radio value="light" className="hover:bg-slate-500">
  //             Light
  //           </Dropdown.Radio>
  //         </Dropdown.RadioGroup>
  //         <Dropdown.Separator />
  //         <Link to="/logout">
  //           <Dropdown.Item
  //             key="logout"
  //             icon={<LogOutIcon size={14} />}
  //             className="hover:bg-slate-500"
  //           >
  //             Logout
  //           </Dropdown.Item>
  //         </Link>
  //       </div>
  //     }
  //   >
  //     <AvatarTrigger>
  //       <UserCircle2Icon size={20} className="my-1 cursor-pointer" />
  //     </AvatarTrigger>
  //   </Dropdown>
  // )
}

export default Footer
