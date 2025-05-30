import { Flex, Separator } from '@md/ui'

import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface CustomerDetailsPanelProps {
    customer: Customer
}

export const CustomerDetailsPanel = ({ customer }: CustomerDetailsPanelProps) => {
    return (
        <Flex direction="column" className="gap-2 w-1/3">
            <Flex direction="column" className="gap-2 p-6">
                <div className="text-lg font-medium">{customer.name}</div>
                <div className="text-muted-foreground text-[13px] mb-3">{customer.alias}</div>
                <FlexDetails title="Legal name" value={customer.name} />
                <FlexDetails title="Email" value={customer.billingEmail} />
                <FlexDetails title="Currency" value={customer.currency} />
                <FlexDetails title="Country" value={customer.billingAddress?.country ?? ''} />
                <Flex align="center" justify="between">
                    <div className="text-[13px] text-muted-foreground">Address</div>
                    <div className="text-[13px]">{customer.billingAddress?.city}</div>
                </Flex>
                <FlexDetails title="Tax rate" value="Standard" />
                <FlexDetails title="Tax ID" value="None" />
            </Flex>
            <Separator className="-my-3" />
            <Flex direction="column" className="gap-2 p-6">
                <div className="text-[15px] font-medium">Integrations</div>
                <FlexDetails title="Alias (External ID)" value={customer.alias} />
                {/* TODO <FlexDetails title="Hubspot ID" value={customer.connectionMetadata?.hubspot?.0?.externalId} /> */}
                <FlexDetails title="Stripe ID" value="N/A" />
            </Flex>
            <Separator className="-my-3" />
            <Flex direction="column" className="gap-2 p-6">
                <div className="text-[15px] font-medium">Payment</div>
                <FlexDetails title="Payment method" value={customer.currentPaymentMethodId ?? "None"} />
                <FlexDetails title="Payment term" value="N/A" />
                <FlexDetails title="Grace period" value="None" />
            </Flex>
        </Flex>
    )
}

const FlexDetails = ({ title, value }: { title: string; value?: string }) => (
    <Flex align="center" justify="between">
        <div className="text-[13px] text-muted-foreground">{title}</div>
        <div className="text-[13px]">{value ?? 'N/A'}</div>
    </Flex>
) 