import { Tabs, TabsList, TabsTrigger } from '@md/ui'
import { Link } from 'react-router'

import { slotItems } from '@/edition'
import { useBasePath } from '@/hooks/useBasePath'

const QUOTES = 'quotes'

/** Quotes, then the tabs modules add (`quoteList.tabs`); nothing when no module adds one. */
export const QuoteListTabs = ({ active }: { active?: string }) => {
  const basePath = useBasePath()
  const tabs = slotItems('quoteList.tabs')
  if (tabs.length === 0) return null

  return (
    <Tabs value={active ?? QUOTES}>
      <TabsList variant="ghost">
        <TabsTrigger value={QUOTES} asChild>
          <Link to={`${basePath}/quotes`}>Quotes</Link>
        </TabsTrigger>
        {tabs.map(tab => (
          <TabsTrigger key={tab.path} value={tab.path} asChild>
            <Link to={`${basePath}/quotes/${tab.path}`}>{tab.label}</Link>
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  )
}
