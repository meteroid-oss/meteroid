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
  UnplugIcon,
  Edit2Icon,
} from 'lucide-react'
import * as React from 'react'
import { FunctionComponent, useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { siAdyen, siStripe, siHubspot, siQuickbooks } from 'simple-icons'
import { toast } from 'sonner'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import {
  disconnectConnector,
  listConnectors,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { Connector, ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

interface Integration {
  name: string
  description: string
  features: string[]
  disabled?: boolean
  icon?: FunctionComponent<{ className?: string }>
  link?: string
  editLink?: string
  data?: Connector[]
  multiConnectionsDisabled?: boolean
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
    <path d={path} />
  </svg>
)

// Stancer's logo (from https://docs.stancer.com/favicon.svg) is a full
// gradient mark, not a flat single-color path like the simple-icons ones
// above, so it's inlined directly rather than going through BrandIcon.
// Gradient/filter ids are prefixed to avoid colliding with other ids in the DOM.
export const StancerLogo = ({ className }: { className?: string }) => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 104 104" className={className}>
    <defs>
      <linearGradient
        id="stancer-logo-a"
        x1="57.242"
        y1="35"
        x2="1.041"
        y2="80.666"
        gradientUnits="userSpaceOnUse"
      >
        <stop stopColor="#215DD2" />
        <stop offset="1" stopColor="#79A7FF" />
      </linearGradient>
      <linearGradient
        id="stancer-logo-b"
        x1="82.862"
        y1="7.249"
        x2="41.529"
        y2="53.322"
        gradientUnits="userSpaceOnUse"
      >
        <stop stopColor="#d0e9ff" />
        <stop offset="1" stopColor="#FF5B58" />
      </linearGradient>
      <filter
        id="stancer-logo-c"
        x="24.534"
        y="29.266"
        width="58.749"
        height="44.733"
        filterUnits="userSpaceOnUse"
        colorInterpolationFilters="sRGB"
      >
        <feFlood floodOpacity="0" result="BackgroundImageFix" />
        <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
        <feGaussianBlur stdDeviation="2" result="effect1_foregroundBlur" />
      </filter>
    </defs>
    <path
      d="M35.656 32h-11.57C17.239 32 11.28 36.6 9.664 43.131L.667 83.509C-.305 87.873 3.092 92 7.657 92h51.27c4.528 0 8.446-3.09 9.422-7.43l9.973-44.347C79.272 36 76.313 32 71.908 32H35.656Z"
      fill="url(#stancer-logo-a)"
    />
    <path
      d="M51.509 12h46.18c4.352 0 7.587 4 6.649 8.223l-.965 4.343H36.908l.35-1.435C38.856 16.599 44.744 12 51.509 12ZM35.112 31.913l-8.486 34.72C25.958 69.367 28.042 72 30.873 72h54.303c4.473 0 8.344-3.09 9.309-7.43l7.256-32.657H35.112Z"
      fill="#d0e9ff"
      fillRule="evenodd"
    />
    <path
      d="m71.563 70 7.72-36.734H36.695L28.66 64.633c-.668 2.733 1.416 5.366 4.247 5.366h38.656Z"
      fill="url(#stancer-logo-b)"
      fillRule="evenodd"
      filter="url(#stancer-logo-c)"
    />
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
  }, [success])

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
            <BrandIcon path={siStripe.path} color="#635bff" className={className} />
          ),
          link: `add-stripe`,
          data: connectorsQuery.data?.connectors.filter(
            connector => connector.provider === ConnectorProviderEnum.STRIPE
          ),
        },
        {
          name: 'GoCardless',
          description: 'Bank-debit collection across SEPA, BACS, ACH',
          features: ['Direct Debit (SEPA, BACS, ACH)', 'Recurring mandates'],
          // GoCardless brand isn't in simple-icons; fall back to a generic
          // bank glyph (lucide's BanknoteIcon already in scope). Swap for
          // a proper brand SVG when one is available.
          icon: ({ className }) => <BanknoteIcon className={cn(className, 'text-[#5063F0]')} />,
          link: `add-gocardless`,
          data: connectorsQuery.data?.connectors.filter(
            connector => connector.provider === ConnectorProviderEnum.GOCARDLESS
          ),
        },
        {
          name: 'Stancer',
          description: 'European card payments platform',
          features: ['Card'],
          icon: ({ className }) => <StancerLogo className={className} />,
          link: `add-stancer`,
          data: connectorsQuery.data?.connectors.filter(
            connector => connector.provider === ConnectorProviderEnum.STANCER
          ),
        },
        {
          name: 'Adyen',
          description: 'Enterprise payment solution',
          features: ['Card', 'Direct Debit (SEPA, ACH, Bacs)'],
          disabled: true,
          icon: ({ className }) => (
            <BrandIcon path={siAdyen.path} color="#0abf53" className={className} />
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
            <BrandIcon path={siHubspot.path} color="#ff7a59" className={className} />
          ),
          features: [],
          link: 'connect-hubspot',
          editLink: 'edit-hubspot-connection',
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
            <BrandIcon path={siQuickbooks.path} color="#00a550" className={className} />
          ),
          features: [],
          disabled: true,
        },
      ],
    },
  ]

  // TODO, also scroll when reload with #hash
  const handleScroll: React.UIEventHandler<HTMLDivElement> = _e => {}

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
                <h2 className="text-sm font-semibold mb-3 flex items-center text-foreground sticky top-0 bg-background py-1">
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
                                <integration.icon className="w-6 h-6" />
                              ) : (
                                <section.icon className="w-5 h-5 text-foreground" />
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
                                    <CheckCircle className="w-3 h-3 text-primary mr-1" />
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
                                <PlugIcon className="w-3 h-3 ml-2" />{' '}
                              </Link>
                            </Button>
                          ) : (
                            <div className="flex flex-col items-end gap-2">
                              <div className="flex items-center gap-2">
                                <span className="text-xs text-success flex items-center gap-1.5">
                                  <CheckCircle2 className="w-4 h-4" /> Connected
                                </span>

                                {integration.multiConnectionsDisabled ? (
                                  <Button
                                    size="icon"
                                    variant="ghost"
                                    className="font-semibold"
                                    disabled
                                  >
                                    <PlusIcon size={16} />
                                  </Button>
                                ) : (
                                  <Button
                                    size="icon"
                                    variant="ghost"
                                    className="font-semibold"
                                    asChild
                                  >
                                    <Link to={integration.link ?? '#'}>
                                      <PlusIcon size={16} />
                                    </Link>
                                  </Button>
                                )}
                              </div>
                              {integration.data.map(connector => (
                                <div
                                  key={connector.id}
                                  className="flex items-center gap-2"
                                >
                                  <span className="text-xs">
                                    <CopyToClipboardButton text={connector.alias} />
                                  </span>
                                  <Popover>
                                    <PopoverTrigger className="flex items-center justify-center w-9">
                                      <MoreVerticalIcon size={16} className="cursor-pointer" />
                                    </PopoverTrigger>
                                    <PopoverContent
                                      className="p-0 w-32"
                                      side="bottom"
                                      align="end"
                                    >
                                        {integration.editLink && (
                                          <Link
                                            to={`${integration.editLink}/${connector.id}`}
                                            className="w-full text-xs"
                                          >
                                            <Button
                                              type="button"
                                              variant="ghost"
                                              className="w-full text-xs"
                                            >
                                              <Edit2Icon size={14} className="mr-1" />
                                              Edit
                                            </Button>
                                          </Link>
                                        )}
                                        <Button
                                          type="button"
                                          variant="destructiveGhost"
                                          className="w-full text-xs"
                                          onClick={() => removeConnection(connector.id)}
                                        >
                                          <UnplugIcon size={14} className="mr-1" /> Disconnect
                                        </Button>
                                      </PopoverContent>
                                    </Popover>
                                  </div>
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
                  <Icon className="w-4 h-4 mr-2" />
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
