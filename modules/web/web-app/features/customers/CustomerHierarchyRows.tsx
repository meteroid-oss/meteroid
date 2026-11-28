import { Fragment } from 'react'
import { Link } from 'react-router'

import { DetailRow } from '@/components/layouts/DetailLayout'
import { Slot, hasSlot } from '@/edition'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { getCustomerHierarchy } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'

/** The customer's parent (with its own ancestors) and its child accounts. */
export const CustomerHierarchyRows = ({ customerId }: { customerId: string }) => {
  const basePath = useBasePath()
  const query = useQuery(getCustomerHierarchy, { customerId })
  const ancestors = query.data?.hierarchy?.ancestors ?? []
  const children = query.data?.hierarchy?.children ?? []
  const editable = hasSlot('customerDetail.parentActions')

  const link = (c: { id: string; name: string }) => (
    <Link to={`${basePath}/customers/${c.id}`} className="text-brand hover:underline">
      {c.name}
    </Link>
  )

  return (
    <>
      {(editable || ancestors.length > 0) && (
        <DetailRow
          title="Parent"
          value={
            <span className="flex items-center gap-1 text-[13px]">
              {ancestors.length === 0
                ? '-'
                : [...ancestors].reverse().map((a, i) => (
                    <Fragment key={a.id}>
                      {i > 0 && <span className="text-muted-foreground">/</span>}
                      {link(a)}
                    </Fragment>
                  ))}
              <Slot
                name="customerDetail.parentActions"
                customerId={customerId}
                parentId={ancestors[0]?.id}
              />
            </span>
          }
        />
      )}
      {children.length > 0 && (
        <DetailRow
          title="Accounts"
          value={
            <span className="flex max-w-[250px] flex-col items-end text-[13px]">
              {children.map(c => (
                <Fragment key={c.id}>{link(c)}</Fragment>
              ))}
            </span>
          }
        />
      )}
    </>
  )
}
