import { D, G } from '@mobily/ts-belt'
import { Button, Dropdown } from '@ui/components'
import { PlusIcon } from 'lucide-react'

import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  setStatus: (search: InvoiceStatus) => void
  status?: InvoiceStatus
}

export const FilterDropdown = ({ status, setStatus }: Props) => {
  return (
    <Dropdown
      side="bottom"
      align="start"
      overlay={[
        <Dropdown.RadioGroup
          key="status"
          value={status ? InvoiceStatus[status] : ''}
          onChange={(value: keyof typeof InvoiceStatus) => setStatus(InvoiceStatus[value])}
        >
          {D.values(InvoiceStatus)
            .filter(G.isString)
            .map(status => (
              <Dropdown.Radio key={status} value={status.toString()}>
                {status}
              </Dropdown.Radio>
            ))}
        </Dropdown.RadioGroup>,
      ]}
    >
      <Button variant="tertiary" transparent>
        <PlusIcon size={12} />
        Filter
      </Button>
    </Dropdown>
  )
}
