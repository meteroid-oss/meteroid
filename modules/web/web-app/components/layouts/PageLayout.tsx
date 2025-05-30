import { Flex } from '@ui/index'
import { PropsWithChildren, ReactNode } from 'react'
import { useSearchParams } from 'react-router-dom'

import { ButtonTabs } from '@/components/ButtonTabs'

interface TabConfig {
    key: string
    label: string
}

interface PageLayoutProps extends PropsWithChildren {
    imgLink: 'customers' | 'invoices' | 'subscriptions'
    title: string
    tabs?: TabConfig[]
    customTabs?: ReactNode
    actions?: ReactNode
}

export const PageLayout = ({ imgLink, title, children, tabs, customTabs, actions }: PageLayoutProps) => {
    const [searchParams, setSearchParams] = useSearchParams()
    const currentTab = searchParams.get('tab') || (tabs?.[0]?.key ?? 'all')

    const updateTab = (tab: string) => {
        const newSearchParams = new URLSearchParams(searchParams)

        if (tab === (tabs?.[0]?.key ?? 'all')) {
            newSearchParams.delete('tab')
        } else {
            newSearchParams.set('tab', tab.toLowerCase())
        }
        setSearchParams(newSearchParams)
    }

    const renderTabs = () => {
        if (customTabs) {
            return customTabs
        }

        if (tabs && tabs.length > 0) {
            return (
                <Flex align="center" className="gap-2 ml-2 mt-[0.5px]">
                    {tabs.map((tab) => (
                        <ButtonTabs
                            key={tab.key}
                            active={currentTab === tab.key}
                            onClick={() => updateTab(tab.key)}
                        >
                            {tab.label}
                        </ButtonTabs>
                    ))}
                </Flex>
            )
        }

        return null
    }

    return <main className="flex  flex-col flex-1 w-full max-w-screen-2xl mx-auto h-full overflow-x-hidden">
        <div className="relative pt-4 px-4 h-full overflow-y-auto flex flex-col gap-5">
            <Flex direction="column" className="gap-2 h-full">
                <Flex align="center" justify="between">
                    <Flex align="center" className='gap-2'>
                        <img src={`/header/${imgLink}.svg`} alt={title} />
                        <div className="text-[15px] font-medium">{title}</div>
                        {renderTabs()}
                    </Flex>
                    {actions && <Flex align="center" className='gap-2'>
                        {actions}
                    </Flex>}
                </Flex>
                {children}
            </Flex>
        </div>
    </main>
}