import { SettingsIcon } from '@md/icons'
import { Dropdown } from '@md/ui'
import { LogOutIcon, TerminalIcon, UserCircle2Icon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { useTheme } from 'providers/ThemeProvider'

import { StyledItems as Items } from '../Items/Items.styled'
import Item from '../Items/components/Item/Item'

import { AvatarTrigger, StyledFooter } from './Footer.styled'

import type { FunctionComponent } from 'react'

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

export const FooterAccountDropdown: FunctionComponent = () => {
  const { isDarkMode, setDarkMode } = useTheme()

  return (
    <Dropdown
      side="right"
      align="start"
      className="mb-2"
      overlay={
        <div className="pl-4">
          <Link to="/account/me">
            <Dropdown.Item
              key="header"
              icon={<SettingsIcon size={14} />}
              className="hover:bg-slate-500"
            >
              Account Preferences
            </Dropdown.Item>
          </Link>
          <Dropdown.Separator />
          <Dropdown.Label>Theme</Dropdown.Label>
          <Dropdown.RadioGroup
            key="theme"
            value={isDarkMode ? 'dark' : 'light'}
            onChange={e => setDarkMode(e === 'dark')}
          >
            <Dropdown.Radio value="dark" className="hover:bg-slate-500">
              Dark
            </Dropdown.Radio>
            <Dropdown.Radio value="light" className="hover:bg-slate-500">
              Light
            </Dropdown.Radio>
          </Dropdown.RadioGroup>
          <Dropdown.Separator />
          <Link to="/logout">
            <Dropdown.Item
              key="logout"
              icon={<LogOutIcon size={14} />}
              className="hover:bg-slate-500"
            >
              Logout
            </Dropdown.Item>
          </Link>
        </div>
      }
    >
      <AvatarTrigger>
        <UserCircle2Icon size={20} className="my-1 cursor-pointer" />
      </AvatarTrigger>
    </Dropdown>
  )
}

export default Footer
