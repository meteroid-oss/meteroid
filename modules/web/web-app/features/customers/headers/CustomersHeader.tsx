import { SearchIcon } from '@md/icons'
import { Button, ButtonProps, InputWithIcon, Flex as NewFlex, Separator, cn } from '@md/ui'
import { FileUpIcon, ListFilter } from 'lucide-react'
import { FunctionComponent, PropsWithChildren, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { CustomersExportModal } from '@/features/customers/modals/CustomersExportModal'
import { CustomersImportModal } from '@/features/customers/modals/CustomersImportModal'

interface CustomersHeaderProps {
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (search: string) => void
  search: string
  onImportSuccess?: () => void
}

export const CustomersHeader: FunctionComponent<CustomersHeaderProps> = ({
  setEditPanelVisible,
  setSearch,
  search,
  onImportSuccess,
}) => {
  const [searchParams, setSearchParams] = useSearchParams()
  const currentTab = searchParams.get('tab') || 'active'

  const [visible, setVisible] = useState(false)
  const [importVisible, setImportVisible] = useState(false)

  const updateTab = (tab: string) => {
    const newSearchParams = new URLSearchParams(searchParams)

    if (tab === 'active') newSearchParams.delete('tab')
    else {
      newSearchParams.set('tab', tab.toLowerCase())
    }
    setSearchParams(newSearchParams)
  }

  return (
    <>
      <div className="flex flex-col gap-2">
        <div className="flex flex-row items-center justify-between">
          <NewFlex align="center" className="gap-2">
            <img src="/header/customer.svg" alt="customer logo"/>
            <div className="text-[15px] font-medium">Customers</div>
            <NewFlex align="center" className="gap-2 ml-2 mt-[0.5px]">
              <ButtonTabs active={currentTab === 'active'} onClick={() => updateTab('active')}>
                Active
              </ButtonTabs>
              <ButtonTabs active={currentTab === 'archived'} onClick={() => updateTab('archived')}>
                Archived
              </ButtonTabs>
            </NewFlex>
          </NewFlex>
          <div className="flex flex-row gap-2">
            <Button size="sm" onClick={() => setVisible(true)} variant="secondary">
              Export
            </Button>
            <Button variant="secondary" size="sm" onClick={() => setImportVisible(true)}>
              <FileUpIcon className="h-4 w-4 mr-2"/>
              Import CSV
            </Button>
            <Button size="sm" variant="default" onClick={() => setEditPanelVisible(true)}>
              New customer
            </Button>
          </div>
        </div>
        <div className="mx-[-16px]">
          <Separator/>
        </div>
        <div className="flex flex-row items-center gap-2">
          <InputWithIcon
            className="h-[30px]"
            placeholder="Search..."
            icon={<SearchIcon size={16} className="text-[#898784]"/>}
            width="fit-content"
            value={search}
            onChange={e => setSearch(e.target.value)}
          />
          <Button
            hasIcon
            className="h-[30px] bg-accent text-accent-foreground hover:opacity-90"
            variant="outline"
          >
            <ListFilter size={16} className="text-[#898784]"/> Filter
          </Button>
        </div>
      </div>
      <CustomersExportModal openState={[visible, setVisible]}/>
      <CustomersImportModal openState={[importVisible, setImportVisible]} onSuccess={onImportSuccess}/>
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
