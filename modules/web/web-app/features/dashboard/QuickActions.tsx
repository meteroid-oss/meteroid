import { useState } from 'react'
import { Link } from 'react-router'

import { Gate } from '@/components/Gate'
import { PillPlusIcon } from '@/components/icons/AppIcons'
import { CustomersCreatePanel } from '@/features/customers'
import { useBasePath } from '@/hooks/useBasePath'

// One look for every pill: white at 10% over the page (#272727).
const PILL =
  'inline-flex h-8 items-center gap-2 rounded-full bg-foreground/10 pl-3 pr-4 text-[13px] leading-5 font-medium text-sidebar-accent-foreground/90 transition-colors hover:bg-foreground/15 [&_svg]:text-faint-foreground'

const LINKS = [
  { label: 'Invoice', to: 'invoices/create', perm: 'billing:invoices:manage' },
  { label: 'Quote', to: 'quotes/create', perm: 'billing:quotes:manage' },
  { label: 'Subscription', to: 'subscriptions/create', perm: 'billing:subscriptions:manage' },
]

/** Home's quick-create pills: a new customer (the customers page's create panel), invoice, quote
 *  or subscription (their create pages). Each shows only with the right to create it. */
export const QuickActions = () => {
  const basePath = useBasePath()
  const [createCustomer, setCreateCustomer] = useState(false)

  return (
    <div className="flex flex-wrap items-center gap-2">
      <Gate perm="customers:manage">
        <button type="button" className={PILL} onClick={() => setCreateCustomer(true)}>
          <PillPlusIcon />
          Customer
        </button>
      </Gate>
      {LINKS.map(link => (
        <Gate key={link.to} perm={link.perm}>
          <Link to={`${basePath}/${link.to}`} className={PILL}>
            <PillPlusIcon />
            {link.label}
          </Link>
        </Gate>
      ))}
      <CustomersCreatePanel visible={createCustomer} closePanel={() => setCreateCustomer(false)} />
    </div>
  )
}
