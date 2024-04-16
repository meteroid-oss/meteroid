import type { FunctionComponent, ReactNode } from 'react'

interface InnerMenuProps {
  title: string
  children: ReactNode
}

const InnerMenu: FunctionComponent<InnerMenuProps> = ({ title, children }) => {
  return (
    <aside className="flex flex-col w-[250px] border-r border-border bg-muted dark:bg-card">
      <header className="pt-6 pb-3 pl-4">
        <h2 className="text-lg font-medium leading-none">{title}</h2>
      </header>

      {children}
    </aside>
  )
}

export default InnerMenu
