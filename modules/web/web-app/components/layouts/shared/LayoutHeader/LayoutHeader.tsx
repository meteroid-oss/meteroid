import { colors } from '@md/foundation'
import { ButtonAlt } from '@md/ui'
import * as Tooltip from '@radix-ui/react-tooltip'
import { Command as IconCommand, Search as IconSearch } from 'lucide-react'

import { useOrganization } from '@/hooks/useOrganization'
import { useProductFamily } from '@/hooks/useProductFamily'
import { useTenant } from '@/hooks/useTenant'
import { detectOS } from '@/lib/helpers'

import { Breadcrumb } from './BreadcrumbsView'
import HelpPopover from './HelpPopover'
import { TenantDropdown } from './TenantDropdown'
import { ThemeSwitch } from './ThemeSwitch'

interface LayoutHeaderProps {
  customHeaderComponents?: React.ReactNode
  breadcrumbs?: Breadcrumb[]
  headerBorder?: boolean
}
export const LayoutHeader = ({
  customHeaderComponents,
  breadcrumbs = [],
  headerBorder = false,
}: LayoutHeaderProps) => {
  const { tenant } = useTenant()
  const { organization } = useOrganization()

  const os = detectOS()

  const { productFamily } = useProductFamily()

  const allBreadcrumbs: Breadcrumb[] = productFamily
    ? [
        ...breadcrumbs,
        {
          key: 'product-family',
          label: productFamily.name,
        },
      ]
    : breadcrumbs

  return (
    <div
      className={`flex  items-center justify-between py-4 pr-5 pl-10 ${
        headerBorder ? 'border-b border-scale-500' : ''
      }`}
    >
      <div className="flex items-center text-sm">
        <TenantDropdown />
      </div>
      <div className="flex items-center space-x-2">
        {customHeaderComponents && customHeaderComponents}
        <Tooltip.Provider>
          <Tooltip.Root delayDuration={0}>
            <Tooltip.Trigger asChild>
              <div className="flex">
                <ButtonAlt
                  type="default"
                  icon={<IconSearch size={16} strokeWidth={1.5} className="text-scale-1200" />}
                  onClick={() => alert('Not implemented')}
                />
              </div>
            </Tooltip.Trigger>
            <Tooltip.Portal>
              <Tooltip.Content side="bottom">
                <Tooltip.Arrow fill={colors.neutral2} />
                <div
                  className={[
                    'rounded bg-scale-100 py-1 px-2 leading-none shadow',
                    'border border-scale-200 flex items-center space-x-1',
                  ].join(' ')}
                >
                  {os === 'macos' ? (
                    <IconCommand size={11.5} strokeWidth={1.5} className="text-scale-1200" />
                  ) : (
                    <p className="text-xs">CTRL</p>
                  )}
                  <p className="text-xs">K</p>
                </div>
              </Tooltip.Content>
            </Tooltip.Portal>
          </Tooltip.Root>
        </Tooltip.Provider>
        <HelpPopover />
        <ThemeSwitch />
      </div>
    </div>
  )
}
