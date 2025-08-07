import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Card,
  CardContent,
  Popover,
  PopoverContent,
  PopoverTrigger,
  ScrollArea,
} from '@ui/components'
import { cn } from '@ui/lib'
import {
  BanknoteIcon,
  CheckCircle,
  CheckCircle2,
  CreditCard,
  MoreVerticalIcon,
  PlugIcon,
  PlusIcon,
  Users,
} from 'lucide-react'
import * as React from 'react';
import { FunctionComponent, useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { siAdyen, siStripe, siHubspot, siQuickbooks } from 'simple-icons'
import { toast } from 'sonner'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { useQueryState } from "@/hooks/useQueryState";
import { useQuery } from '@/lib/connectrpc'
import {
  disconnectConnector,
  listConnectors,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { Connector, ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import { getInstance } from "@/rpc/api/instance/v1/instance-InstanceService_connectquery";
import { useConfirmationModal } from 'providers/ConfirmationProvider'


interface Integration {
  name: string
  description: string
  features: string[]
  disabled?: boolean
  icon?: FunctionComponent<{ className?: string }>
  link?: string
  data?: Connector[],
  multiConnectionsDisabled?: boolean,
}

interface Section {
  id: string
  title: string
  icon: FunctionComponent<{ className?: string }>
  integrations: Integration[]
}

export const BrandIcon = ({
  path,
  color,
  className,
}: {
  path: string
  color: string
  className?: string
}) => (
  <svg viewBox="0 0 24 24" fill={color} className={className}>
    <path d={path}/>
  </svg>
)

export const IntegrationsTab = () => {
  // TODO set based on #hash
  const [activeSection] = useState('')
  const [success] = useQueryState<boolean | undefined>('success', undefined)

  useEffect(() => {
    if (success) {
      toast.success('Connected!', { id: 'integration-success-toast' })
    }
  }, [success]);

  const connectorsQuery = useQuery(listConnectors, {})

  const disconnectConnectorMutation = useMutation(disconnectConnector, {
    onSuccess: () => {
      connectorsQuery.refetch()
    },
  })

  const getInstanceQuery = useQuery(getInstance)

  const sections: Section[] = [
    {
      id: 'payment-providers',
      title: 'Payment Providers',
      icon: CreditCard,
      integrations: [
        {
          name: 'Stripe',
          description: 'Global payments platform',
          features: ['Card', 'Direct Debit (SEPA, ACH, Bacs)', 'Link'],
          icon: ({ className }) => (
            <BrandIcon path={siStripe.path} color="#635bff" className={className}/>
          ),
          link: `add-stripe`,
          data: connectorsQuery.data?.connectors.filter(
            connector => connector.provider === ConnectorProviderEnum.STRIPE
          ),
        },
        {
          name: 'Adyen',
          description: 'Enterprise payment solution',
          features: ['Card', 'Direct Debit (SEPA, ACH, Bacs)'],
          disabled: true,
          icon: ({ className }) => (
            <BrandIcon path={siAdyen.path} color="#0abf53" className={className}/>
          ),
        },
      ],
    },
    {
      id: 'crm',
      title: 'CRM',
      icon: Users,
      integrations: [
        {
          name: 'HubSpot',
          description: 'Marketing & sales platform',
          icon: ({ className }) => (
            <BrandIcon path={siHubspot.path} color="#ff7a59" className={className}/>
          ),
          features: [],
          link: 'connect-hubspot',
          data: connectorsQuery.data?.connectors.filter(
            connector => connector.provider === ConnectorProviderEnum.HUBSPOT
          ),
          disabled: !getInstanceQuery.data?.hubspotOauthClientId,
          multiConnectionsDisabled: true,
        },
      ],
    },
    {
      id: 'accounting',
      title: 'Accounting',
      icon: BanknoteIcon,
      integrations: [
        {
          name: 'Pennylane',
          description: 'Financial and accounting management',
          features: [],
          link: 'connect-pennylane',
          data: connectorsQuery.data?.connectors.filter(
            connector => connector.provider === ConnectorProviderEnum.PENNYLANE
          ),
          disabled: !getInstanceQuery.data?.pennylaneOauthClientId,
          multiConnectionsDisabled: true,
        },
        {
          name: 'Quickbooks',
          description: 'Accounting software by Intuit',
          icon: ({ className }) => (
            <BrandIcon path={siQuickbooks.path} color="#00a550" className={className}/>
          ),
          features: [],
          disabled: true,
        },
      ],
    },
  ]

  // TODO, also scroll when reload with #hash
  const handleScroll: React.UIEventHandler<HTMLDivElement> = _e => {
  }

  const showConfirmationModal = useConfirmationModal()

  const removeConnection = async (id: string) => {
    showConfirmationModal(() => disconnectConnectorMutation.mutate({ id }))
  }

  return (
    <div className="mx-auto flex">
      {/* Main Content Area */}
      <div className="flex-1 flex flex-col min-h-0">
        {' '}
        {/* min-h-0 is crucial for nested flex scroll */}
        {/* Fixed Header */}
        <div className="flex-none p-6 pb-4 border-b">
          <h1 className="text-2xl font-semibold mb-2 text-foreground">Integrations</h1>
          <p className="text-muted-foreground text-sm">Connect your favorite tools and services</p>
        </div>
        <ScrollArea className="h-[calc(100vh-280px)]">
          {/* Scrollable Integration List */}
          <div className="flex-1   px-6 py-4" onScroll={handleScroll}>
            {sections.map(section => (
              <section key={section.id} id={section.id} className="mb-6 last:mb-0">
                <h2
                  className="text-sm font-semibold mb-3 flex items-center text-foreground sticky top-0 bg-background py-1">
                  {section.title}
                </h2>
                <div className="grid gap-3">
                  {section.integrations.map(integration => (
                    <Card
                      key={integration.name}
                      className="overflow-hidden hover:shadow-sm transition-shadow"
                    >
                      <CardContent
                        className={cn(integration.disabled && 'bg-secondary', 'p-4 group')}
                      >
                        <div className="flex items-center justify-between">
                          <div className="flex items-center space-x-4">
                            <div className="bg-muted p-2 rounded-md">
                              {integration.icon ? (
                                <integration.icon className="w-6 h-6"/>
                              ) : (
                                <section.icon className="w-5 h-5 text-foreground"/>
                              )}
                            </div>
                            <div className="space-y-2">
                              <h3 className="text-sm font-medium text-foreground">
                                {integration.name}
                              </h3>
                              <p className="text-sm text-muted-foreground">
                                {integration.description}
                              </p>
                              <div className="flex gap-3">
                                {integration.features.map(feature => (
                                  <div
                                    key={feature}
                                    className="flex items-center text-xs text-muted-foreground"
                                  >
                                    <CheckCircle className="w-3 h-3 text-primary mr-1"/>
                                    {feature}
                                  </div>
                                ))}
                              </div>
                            </div>
                          </div>
                          {integration.disabled ? (
                            <span className="text-xs text-muted-foreground pr-4">Coming soon</span>
                          ) : connectorsQuery.isLoading ? (
                            <></>
                          ) : !integration.data?.length ? (
                            <Button
                              size="sm"
                              variant="brand"
                              className="min-w-[100px] font-semibold"
                              asChild
                            >
                              <Link to={integration.link ?? '#'}>
                                Connect
                                <PlugIcon className="w-3 h-3 ml-2"/>{' '}
                              </Link>
                            </Button>
                          ) : (
                            <div className="  grid grid-cols-[1fr,auto] gap-2">
                              <span className="text-xs text-success flex items-center gap-2  ">
                                <CheckCircle2/> Connected
                              </span>

                              {integration.multiConnectionsDisabled ? (
                                <Button
                                  size="icon"
                                  variant="ghost"
                                  className="font-semibold"
                                  disabled
                                >
                                  <PlusIcon size={16}/>
                                </Button>
                              ) : (
                                <Button
                                  size="icon"
                                  variant="ghost"
                                  className="font-semibold"
                                  asChild
                                >
                                  <Link to={integration.link ?? '#'}>
                                    <PlusIcon size={16}/>{' '}
                                  </Link>
                                </Button>
                              )}
                              {integration.data.map(connector => (
                                <React.Fragment key={connector.id}>
                                  <span className="text-xs">
                                    <CopyToClipboardButton text={connector.alias}/>
                                  </span>
                                  <span className="items-center justify-self-end">
                                    <Popover>
                                      <PopoverTrigger className="items-center justify-items-center w-9 ">
                                        <MoreVerticalIcon size={16} className="cursor-pointer"/>
                                      </PopoverTrigger>
                                      <PopoverContent
                                        className="p-0  w-24  "
                                        side="bottom"
                                        align="end"
                                      >
                                        <Button
                                          type="button"
                                          variant="destructiveGhost"
                                          className=" w-full text-xs"
                                          onClick={() => removeConnection(connector.id)}
                                        >
                                          Disconnect
                                        </Button>
                                      </PopoverContent>
                                    </Popover>
                                  </span>
                                </React.Fragment>
                              ))}
                            </div>
                          )}
                        </div>
                      </CardContent>
                    </Card>
                  ))}
                </div>
              </section>
            ))}
          </div>
        </ScrollArea>
      </div>

      {/* Table of Contents */}
      <div className="w-64 border-l flex-none lg:block hidden">
        <div className="p-6">
          <h2 className="text-xs font-semibold mb-4 text-muted-foreground uppercase tracking-wider">
            Contents
          </h2>
          <nav className="space-y-1">
            {sections.map(section => {
              const Icon = section.icon
              return (
                <a
                  key={section.id}
                  href={`#${section.id}`}
                  className={`
                    flex items-center px-3 py-2 text-sm rounded-md transition-colors
                    ${
                    activeSection === section.id
                      ? 'bg-accent text-accent-foreground'
                      : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground'
                  }
                  `}
                >
                  <Icon className="w-4 h-4 mr-2"/>
                  {section.title}
                </a>
              )
            })}
          </nav>
        </div>
      </div>
    </div>
  )
}
