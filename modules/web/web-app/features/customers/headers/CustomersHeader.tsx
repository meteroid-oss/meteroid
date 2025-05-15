import { spaces } from '@md/foundation'
import { SearchIcon } from '@md/icons'
import { Button, ButtonProps, InputWithIcon, Flex as NewFlex, Separator, cn } from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { ListFilter } from 'lucide-react'
import { FunctionComponent, PropsWithChildren, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { CustomersExportModal } from '@/features/customers/modals/CustomersExportModal'

interface CustomersHeaderProps {
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (search: string) => void
  search: string
}

export const CustomersHeader: FunctionComponent<CustomersHeaderProps> = ({
  setEditPanelVisible,
  setSearch,
  search,
}) => {
  const [searchParams, setSearchParams] = useSearchParams()
  const currentTab = searchParams.get('tab') || 'all'

  const [visible, setVisible] = useState(false)

  const updateTab = (tab: string) => {
    const newSearchParams = new URLSearchParams(searchParams)

    if (tab === 'all') newSearchParams.delete('tab')
    else {
      newSearchParams.set('tab', tab.toLowerCase())
    }
    setSearchParams(newSearchParams)
  }

  return (
    <>
      <Flex direction="column" gap={spaces.space4}>
        <Flex direction="row" align="center" justify="space-between">
          <NewFlex align="center" className="gap-2">
            <img src="/header/customer.svg" alt="customer logo" />
            <div className="text-[15px] font-medium">Customers</div>
            <NewFlex align="center" className="gap-2 ml-2 mt-[0.5px]">
              <ButtonTabs active={currentTab === 'all'} onClick={() => updateTab('all')}>
                All
              </ButtonTabs>
              <ButtonTabs active={currentTab === 'active'} onClick={() => updateTab('active')}>
                Active
              </ButtonTabs>
              <ButtonTabs active={currentTab === 'inactive'} onClick={() => updateTab('inactive')}>
                Inactive
              </ButtonTabs>
              <ButtonTabs active={currentTab === 'archived'} onClick={() => updateTab('archived')}>
                Archived
              </ButtonTabs>
            </NewFlex>
          </NewFlex>
          <Flex direction="row" gap={spaces.space4}>
            <Button size="sm" onClick={() => setVisible(true)} variant="secondary">
              Export
            </Button>
            <Button size="sm" variant="default" onClick={() => setEditPanelVisible(true)}>
              New customer
            </Button>
          </Flex>
        </Flex>
        <div className="mx-[-16px]">
          <Separator />
        </div>
        <Flex direction="row" align="center" gap={spaces.space4}>
          <InputWithIcon
            className="h-[30px]"
            placeholder="Search..."
            icon={<SearchIcon size={16} className="text-[#898784]" />}
            width="fit-content"
            value={search}
            onChange={e => setSearch(e.target.value)}
          />
          <Button
            hasIcon
            className="h-[30px] bg-accent text-accent-foreground hover:opacity-90"
            variant="outline"
          >
            <ListFilter size={16} className="text-[#898784]" /> Filter
          </Button>
        </Flex>
      </Flex>
      <CustomersExportModal openState={[visible, setVisible]} />
    </>
  )
}

interface ButtonTabsProps extends Omit<ButtonProps, 'variant'>, PropsWithChildren {
  active?: boolean
}

const ButtonTabs = ({ children, active = false, ...props }: ButtonTabsProps) => {
  const { className, ...rest } = props

  return (
    <Button
      variant="ghost"
      className={cn(
        'text-[#606060] px-2 h-[26px] text-xs',
        active && 'bg-accent text-accent-foreground',
        !active && 'hover:bg-accent hover:text-accent-foreground',
        className
      )}
      {...rest}
    >
      {children}
    </Button>
  )
}
