import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import {
  Button,
  Popover,
  PopoverContent,
  PopoverTrigger,
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
  Users, UnplugIcon, Edit2Icon,
} from 'lucide-react'
import * as React from 'react';
import { FunctionComponent, useEffect } from 'react'
import { Link } from 'react-router-dom'
import { siAdyen, siHubspot, siQuickbooks } from 'simple-icons'
import { toast } from 'sonner'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import {
  useConnectorLifecycle,
  useConnectorPaths,
} from '@/features/payments/ConnectorLifecycle'
import {
  ConnectorStatusBadge,
  CredentialsErrorBadge,
  MethodChips,
  methodChips,
  userFacingError,
} from '@/features/payments/connectors'
import { BrandIcon } from '@/features/payments/logos'
import {
  MOCK_PAYMENT_PROVIDER,
  PAYMENT_PROVIDERS,
  PaymentProviderDefinition,
} from '@/features/payments/providers'
import {
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useQueryState } from "@/hooks/useQueryState";
import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteConnector,
  listConnectors,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import {
  Connector,
  ConnectorProviderEnum,
  ConnectorStatusEnum,
} from '@/rpc/api/connectors/v1/models_pb'
import { getInstance } from "@/rpc/api/instance/v1/instance-InstanceService_connectquery";
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'


interface Integration {
  name: string
  description: string
  features: string[]
  disabled?: boolean
  icon?: FunctionComponent<{ className?: string }>
  link?: string
  editLink?: string
  data?: Connector[],
  multiConnectionsDisabled?: boolean,
  /** Payment providers list each connection under the provider row. */
  paymentProvider?: PaymentProviderDefinition
}

interface Section {
  id: string
  title: string
  icon: FunctionComponent<{ className?: string }>
  integrations: Integration[]
}

export const IntegrationsTab = () => {
  const [success] = useQueryState<boolean | undefined>('success', undefined)

  useEffect(() => {
    if (success) {
      toast.success('Connected!', { id: 'integration-success-toast' })
    }
  }, [success]);

  const connectorsQuery = useQuery(listConnectors, {})

  const queryClient = useQueryClient()
  const deleteConnectorMutation = useMutation(deleteConnector, {
    onSuccess: () => {
      toast.success('Disconnected')
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listConnectors, cardinality: undefined }),
      })
    },
    onError: error => {
      toast.error(userFacingError(error, 'Failed to disconnect.'))
    },
  })

  const lifecycle = useConnectorLifecycle()

  const getInstanceQuery = useQuery(getInstance)
  const { tenant } = useTenant()

  // The backend refuses the test gateway on production tenants; hide it there too. An existing
  // connection stays listed so it can be managed.
  const mockAvailable = tenant?.environment !== TenantEnvironmentEnum.PRODUCTION
  const hasMockConnector = !!connectorsQuery.data?.connectors.some(
    connector => connector.paymentCapabilities?.isTestGateway
  )
  const paymentProviders =
    mockAvailable || hasMockConnector
      ? [...PAYMENT_PROVIDERS, MOCK_PAYMENT_PROVIDER]
      : PAYMENT_PROVIDERS

  const sections: Section[] = [
    {
      id: 'payment-providers',
      title: 'Payment Providers',
      icon: CreditCard,
      integrations: [
        ...paymentProviders.map<Integration>(provider => ({
          name: provider.name,
          description: provider.description,
          features: provider.features,
          icon: provider.Logo,
          link: `connect-payment-provider/${provider.key}`,
          data: connectorsQuery.data?.connectors.filter(
            connector => connector.provider === provider.provider
          ),
          paymentProvider: provider,
        })),
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
            <BrandIcon path={siQuickbooks.path} color="#00a550" className={className}/>
          ),
          features: [],
          disabled: true,
        },
      ],
    },
  ]

  const showConfirmationModal = useConfirmationModal()

  const removeConnection = async (id: string) => {
    showConfirmationModal(() => deleteConnectorMutation.mutate({ id }))
  }

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      {sections.map((section, sectionIdx) => (
        <SettingsSection
          key={section.id}
          divided={sectionIdx !== 0}
          title={section.title}
          description={
            sectionIdx === 0 ? 'Connect your favorite tools and services.' : undefined
          }
        >
          <SettingsRowGroup>
            {section.integrations.map(integration => (
              <React.Fragment key={integration.name}>
                <SettingsRow
                  className={cn(integration.disabled && 'bg-secondary')}
                  title={
                    <>
                      <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-muted">
                        {integration.icon ? (
                          <integration.icon className="h-4 w-4" />
                        ) : (
                          <section.icon className="h-4 w-4 text-foreground" />
                        )}
                      </span>
                      {integration.name}
                    </>
                  }
                  description={
                    <div className="space-y-1">
                      <div>{integration.description}</div>
                      {integration.features.length > 0 && (
                        <div className="flex flex-wrap gap-x-3 gap-y-1">
                          {integration.features.map(feature => (
                            <span key={feature} className="flex items-center gap-1">
                              <CheckCircle className="h-3 w-3 text-primary" />
                              {feature}
                            </span>
                          ))}
                        </div>
                      )}
                    </div>
                  }
                >
                  {integration.disabled ? (
                    <span className="text-xs text-muted-foreground">Coming soon</span>
                  ) : connectorsQuery.isLoading ? null : !integration.data?.length ? (
                    <Button size="sm" variant="brand" className="font-semibold" asChild>
                      <Link to={integration.link ?? '#'}>
                        Connect
                        <PlugIcon className="ml-2 h-3 w-3" />
                      </Link>
                    </Button>
                  ) : integration.paymentProvider ? (
                    <Button size="sm" variant="secondary" hasIcon asChild>
                      <Link to={integration.link ?? '#'}>
                        <PlusIcon size={14} />
                        Add connection
                      </Link>
                    </Button>
                  ) : (
                    <>
                      <span className="flex items-center gap-1.5 text-xs text-success">
                        <CheckCircle2 className="h-4 w-4" /> Connected
                      </span>
                      {integration.multiConnectionsDisabled ? (
                        <Button size="icon" variant="ghost" className="font-semibold" disabled>
                          <PlusIcon size={16} />
                        </Button>
                      ) : (
                        <Button size="icon" variant="ghost" className="font-semibold" asChild>
                          <Link to={integration.link ?? '#'}>
                            <PlusIcon size={16} />
                          </Link>
                        </Button>
                      )}
                      {integration.data.map(connector => (
                        <React.Fragment key={connector.id}>
                          <span className="text-xs">
                            <CopyToClipboardButton text={connector.alias} />
                          </span>
                          <Popover>
                            <PopoverTrigger className="w-9">
                              <MoreVerticalIcon size={16} className="cursor-pointer" />
                            </PopoverTrigger>
                            <PopoverContent className="w-32 p-0" side="bottom" align="end">
                              {integration.editLink && (
                                <Link
                                  to={`${integration.editLink}/${connector.id}`}
                                  className="w-full text-xs"
                                >
                                  <Button type="button" variant="ghost" className="w-full text-xs">
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
                        </React.Fragment>
                      ))}
                    </>
                  )}
                </SettingsRow>
                {integration.paymentProvider &&
                  integration.data?.map(connector => (
                    <PaymentConnectionRow
                      key={connector.id}
                      connector={connector}
                      lifecycle={lifecycle}
                    />
                  ))}
              </React.Fragment>
            ))}
          </SettingsRowGroup>
        </SettingsSection>
      ))}

      {lifecycle.dialogs}
    </div>
  )
}

/** One payment connection under its provider row, with its status and lifecycle actions. */
const PaymentConnectionRow = ({
  connector,
  lifecycle,
}: {
  connector: Connector
  lifecycle: ReturnType<typeof useConnectorLifecycle>
}) => {
  const paths = useConnectorPaths()
  const isDraft = connector.status === ConnectorStatusEnum.DRAFT
  const isDisabled = connector.status === ConnectorStatusEnum.DISABLED

  return (
    <div
      className={cn(
        'flex items-center gap-3 border-t border-border py-2 pl-[3.25rem] pr-4 text-xs',
        isDisabled && 'bg-muted/40'
      )}
    >
      <span className={cn('font-mono text-[13px]', isDisabled && 'text-muted-foreground')}>
        {connector.alias}
      </span>
      <ConnectorStatusBadge status={connector.status} />
      <CredentialsErrorBadge connector={connector} />
      {!isDraft && (
        <MethodChips chips={methodChips(connector)} className={cn(isDisabled && 'opacity-60')} />
      )}
      <div className="ml-auto flex shrink-0 items-center gap-1.5">
        {isDraft ? (
          <>
            <Button size="sm" variant="secondary" asChild>
              <Link to={paths.resume(connector)}>Resume setup</Link>
            </Button>
            <Button size="sm" variant="ghost" onClick={() => lifecycle.discardDraft(connector)}>
              Discard
            </Button>
          </>
        ) : (
          // Every other action lives in Manage, next to the usage it affects.
          <Button size="sm" variant="secondary" asChild>
            <Link to={paths.manage(connector.id)}>Manage</Link>
          </Button>
        )}
      </div>
    </div>
  )
}
