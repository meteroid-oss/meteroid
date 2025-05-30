import { Card, Flex } from '@md/ui'
import { ChevronDown, Plus } from 'lucide-react'

import { InvoicesCard } from '@/features/customers/cards/InvoicesCard'
import { SubscriptionsCard } from '@/features/customers/cards/SubscriptionsCard'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface CustomerOverviewPanelProps {
    customer: Customer
    onCreateInvoice: () => void
}

export const CustomerOverviewPanel = ({ customer, onCreateInvoice }: CustomerOverviewPanelProps) => {
    return (
        <Flex direction="column" className="gap-4 w-2/3 border-r border-border px-12 py-6">
            <div className="text-lg font-medium">Overview</div>
            <div className="grid grid-cols-2 gap-x-4">
                <OverviewCard title="MRR" value={0} />
                <OverviewCard title="Balance" value={customer?.balanceValueCents ? Number(customer.balanceValueCents) : undefined} />
            </div>
            <Flex align="center" justify="between" className="mt-4">
                <div className="text-lg font-medium">Subscriptions</div>
                <Flex align="center" className="gap-1 text-sm">
                    <Plus size={10} /> Assign subscription
                </Flex>
            </Flex>
            <div className="flex-none">
                <SubscriptionsCard customer={customer} />
            </div>
            <Flex align="center" justify="between" className="mt-4">
                <div className="text-lg font-medium">Invoices</div>
                <Flex
                    align="center"
                    className="gap-1 text-sm cursor-pointer"
                    onClick={onCreateInvoice}
                >
                    <Plus size={10} /> Create invoice
                </Flex>
            </Flex>
            <div className="flex-none">
                <InvoicesCard customer={customer} />
            </div>
        </Flex>
    )
}

const OverviewCard = ({ title, value }: { title: string; value?: number }) => (
    <Card className="bg-[#1A1A1A] bg-gradient-to-t from-[rgba(243,242,241,0.00)] to-[rgba(243,242,241,0.02)] rounded-md p-5">
        <Flex align="center" className="gap-1 text-muted-foreground">
            <div className="text-[13px]">{title}</div>
            <ChevronDown size={10} className="mt-0.5" />
        </Flex>
        <div className="mt-4 text-xl">€ {value}</div>
    </Card>
) 