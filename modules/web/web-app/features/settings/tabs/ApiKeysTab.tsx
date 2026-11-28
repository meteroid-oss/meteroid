import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  AlertDescription,
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertTitle,
  Badge,
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  FieldLabel,
  Input,
  InputWithIcon,
  Label,
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Textarea,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import {
  CheckIcon,
  ChevronRightIcon,
  CopyIcon,
  KeyRoundIcon,
  PlusIcon,
  RefreshCwIcon,
  Trash2Icon,
} from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { toast } from 'sonner'

import {
  AccessLevel,
  ApiKeyKind,
  ApiResourceKey,
  LIFETIMES,
  RESOURCES,
  ROLL_GRACE,
  ResourceLevels,
  describeExpiry,
  grantsAnything,
  invalidOrigins,
  noAccess,
  parseOrigins,
} from '@/features/settings/apiKeys/apiKeyForm'
import { SettingsEmptyState, SettingsSection } from '@/features/settings/components/SettingsSection'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import {
  createApiToken as createApiTokenMutation,
  createPublishableKey as createPublishableKeyMutation,
  listApiTokens,
  revokeApiToken as revokeApiTokenMutation,
  rollApiToken as rollApiTokenMutation,
} from '@/rpc/api/apitokens/v1/apitokens-ApiTokensService_connectquery'
import {
  ApiAccessLevel,
  ApiKeyPermissions,
  ApiResource,
  ApiTokenKind,
} from '@/rpc/api/apitokens/v1/models_pb'
import { parseAndFormatDateTime } from '@/utils/date'

interface CreatedKey {
  kind: ApiKeyKind
  apiKey: string
}

const KINDS: { value: ApiKeyKind; label: string; description: string }[] = [
  {
    value: 'secret',
    label: 'Secret key',
    description: 'For your servers: the whole API. Shown once, never put it in a browser.',
  },
  {
    value: 'publishable',
    label: 'Publishable key',
    description: 'For your website: reads your pricing pages only. Safe in a browser.',
  },
]

const RESOURCE_TO_API: Record<ApiResourceKey, ApiResource> = {
  customers: ApiResource.CUSTOMERS,
  subscriptions: ApiResource.SUBSCRIPTIONS,
  invoices: ApiResource.INVOICES,
  payments: ApiResource.PAYMENTS,
  catalog: ApiResource.CATALOG,
  usage: ApiResource.USAGE,
  entitlements: ApiResource.ENTITLEMENTS,
  webhooks: ApiResource.WEBHOOKS,
  batchJobs: ApiResource.BATCH_JOBS,
}

const LEVEL_TO_API: Record<AccessLevel, ApiAccessLevel> = {
  none: ApiAccessLevel.NONE,
  read: ApiAccessLevel.READ,
  write: ApiAccessLevel.WRITE,
}

const LEVEL_LABEL: Record<ApiAccessLevel, string> = {
  [ApiAccessLevel.NONE]: 'None',
  [ApiAccessLevel.READ]: 'Read',
  [ApiAccessLevel.WRITE]: 'Write',
}

const grantsSummary = (permissions: ApiKeyPermissions) =>
  RESOURCES.map(resource => {
    const grant = permissions.grants.find(g => g.resource === RESOURCE_TO_API[resource.key])
    return { label: resource.label, level: grant?.level ?? ApiAccessLevel.NONE }
  }).filter(g => g.level !== ApiAccessLevel.NONE)

export const ApiKeysTab: FunctionComponent = () => {
  const queryClient = useQueryClient()

  const [displayed, setDisplayed] = useState<CreatedKey>()
  const [loading, setLoading] = useState(false)
  const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false)
  const [tokenName, setTokenName] = useState('')
  const [kind, setKind] = useState<ApiKeyKind>('secret')
  const [originsOpen, setOriginsOpen] = useState(false)
  const [originsText, setOriginsText] = useState('')
  const [tokenToDelete, setTokenToDelete] = useState<{ id: string; name: string } | null>(null)
  const [restricted, setRestricted] = useState(false)
  const [levels, setLevels] = useState<ResourceLevels>(noAccess)
  const [lifetime, setLifetime] = useState('never')
  const [tokenToRoll, setTokenToRoll] = useState<{ id: string; name: string } | null>(null)
  const [rollGrace, setRollGrace] = useState('24')

  const refused = kind === 'publishable' ? invalidOrigins(originsText) : []
  const grantsNothing = kind === 'secret' && restricted && !grantsAnything(levels)

  const resetForm = () => {
    setIsCreateDialogOpen(false)
    setTokenName('')
    setKind('secret')
    setOriginsOpen(false)
    setOriginsText('')
    setRestricted(false)
    setLevels(noAccess())
    setLifetime('never')
  }

  const invalidateList = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listApiTokens,
        cardinality: 'finite',
      }),
    })

  const createTokenMut = useMutation(createApiTokenMutation, {
    onSuccess: async res => {
      await invalidateList()
      setDisplayed({ kind: 'secret', apiKey: res.apiKey })
      resetForm()
    },
    onSettled() {
      setTimeout(() => {
        setLoading(false)
      }, 500)
    },
  })

  const createPublishableMut = useMutation(createPublishableKeyMutation, {
    onSuccess: async res => {
      await invalidateList()
      setDisplayed({ kind: 'publishable', apiKey: res.details?.publishableKey ?? '' })
      resetForm()
    },
    onError: error => {
      toast.error(error.rawMessage || 'Failed to create the publishable key')
    },
    onSettled() {
      setTimeout(() => {
        setLoading(false)
      }, 500)
    },
  })

  const revokeTokenMut = useMutation(revokeApiTokenMutation, {
    onSuccess: async () => {
      await invalidateList()
      toast.success('API key revoked')
      setTokenToDelete(null)
    },
    onError: () => {
      toast.error('Failed to revoke the API key')
    },
  })

  const rollTokenMut = useMutation(rollApiTokenMutation, {
    onSuccess: async res => {
      await invalidateList()
      setDisplayed({ kind: 'secret', apiKey: res.apiKey })
      setTokenToRoll(null)
    },
    onError: error => {
      toast.error(error.rawMessage || 'Failed to roll the API key')
    },
  })

  const tokens = useQuery(listApiTokens)

  const sortedTokens = tokens.data?.apiTokens
    ? [...tokens.data.apiTokens].sort((a, b) => {
        const dateA = a.createdAt ? new Date(a.createdAt).getTime() : 0
        const dateB = b.createdAt ? new Date(b.createdAt).getTime() : 0
        return dateB - dateA
      })
    : []

  const createApiToken = async () => {
    if (!tokenName.trim()) {
      toast.error('Please enter a key name')
      return
    }
    if (refused.length > 0 || grantsNothing) return
    setLoading(true)
    if (kind === 'publishable') {
      createPublishableMut.mutateAsync({
        name: tokenName.trim(),
        allowedOrigins: parseOrigins(originsText),
      })
    } else {
      createTokenMut.mutateAsync({
        name: tokenName.trim(),
        permissions: restricted
          ? {
              grants: RESOURCES.map(resource => ({
                resource: RESOURCE_TO_API[resource.key],
                level: LEVEL_TO_API[levels[resource.key]],
              })),
            }
          : undefined,
        lifetimeDays: LIFETIMES.find(l => l.value === lifetime)?.days,
      })
    }
  }

  const handleDeleteToken = async () => {
    if (!tokenToDelete) return

    revokeTokenMut.mutateAsync({
      id: tokenToDelete.id,
    })
  }

  const now = new Date()

  const copy = (value: string) => copyToClipboard(value, () => toast.success('Copied !'))

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <SettingsSection
        divided={false}
        description="Secret keys call the API from your servers. Publishable keys read your pricing pages from your website."
        action={
          <Button hasIcon onClick={() => setIsCreateDialogOpen(true)} size="sm">
            <PlusIcon size={12} /> Create API key
          </Button>
        }
      >
        <div className="space-y-4">
          {loading && (
            <Alert variant="default">
              <Skeleton height={20} width="100%" />
            </Alert>
          )}
          {!loading && displayed && (
            <Alert variant="success">
              <CheckIcon size={16} />
              <AlertTitle className="pb-2 pt-1">Success !</AlertTitle>
              <AlertDescription className="text-foreground">
                <div className="pb-2">
                  {displayed.kind === 'secret'
                    ? 'This is your API key. Copy it and store it securely, it will not be displayed again'
                    : 'This is your publishable key. It is not a secret: you can copy it again from the list.'}
                </div>
                <InputWithIcon
                  value={displayed.apiKey}
                  readOnly
                  icon={<CopyIcon className="group-hover:text-success" />}
                  className="cursor-pointer"
                  containerClassName="group"
                  onClick={() => copy(displayed.apiKey)}
                />
              </AlertDescription>
            </Alert>
          )}

          {sortedTokens.length === 0 ? (
            <SettingsEmptyState
              icon={<KeyRoundIcon />}
              title="No API keys yet"
              action={
                <Button
                  size="sm"
                  variant="secondary"
                  hasIcon
                  onClick={() => setIsCreateDialogOpen(true)}
                >
                  <PlusIcon size={14} /> Create API key
                </Button>
              }
            >
              Create a secret key for your servers, or a publishable key for your website.
            </SettingsEmptyState>
          ) : (
            <div className="overflow-hidden rounded-lg border border-border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead className="whitespace-nowrap">Key</TableHead>
                    <TableHead className="w-px whitespace-nowrap">Created</TableHead>
                    <TableHead className="w-px whitespace-nowrap">Last used</TableHead>
                    <TableHead className="w-px"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {sortedTokens.map(token => {
                    const publishable = token.kind === ApiTokenKind.PUBLISHABLE
                    const expiry = describeExpiry(token.expiresAt, now)
                    const rollable =
                      !publishable && !token.replacedById && expiry?.tone !== 'expired'
                    return (
                      <TableRow key={token.id}>
                        <TableCell className="max-w-[240px]">
                          <div className="flex items-center gap-2">
                            <span className="truncate font-medium" title={token.name}>
                              {token.name}
                            </span>
                            {publishable && (
                              <Badge variant="secondary" size="sm">
                                Publishable
                              </Badge>
                            )}
                            {token.permissions && (
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <span>
                                    <Badge variant="outline" size="sm">
                                      Restricted
                                    </Badge>
                                  </span>
                                </TooltipTrigger>
                                <TooltipContent>
                                  {grantsSummary(token.permissions).map(g => (
                                    <div key={g.label}>
                                      {g.label}: {LEVEL_LABEL[g.level]}
                                    </div>
                                  ))}
                                </TooltipContent>
                              </Tooltip>
                            )}
                            {token.replacedById && (
                              <Badge variant="secondary" size="sm">
                                Rolled
                              </Badge>
                            )}
                          </div>
                          {expiry && (
                            <div
                              className={cn(
                                'text-xs',
                                expiry.tone === 'muted' && 'text-muted-foreground',
                                expiry.tone === 'warning' && 'text-warning',
                                expiry.tone === 'expired' && 'text-destructive'
                              )}
                            >
                              {expiry.label}
                              {token.lifetimeDays && !token.replacedById
                                ? ` · roll every ${token.lifetimeDays} days`
                                : ''}
                            </div>
                          )}
                          {publishable && (
                            <div
                              className="truncate text-xs text-muted-foreground"
                              title={token.allowedOrigins.join(', ')}
                            >
                              {token.allowedOrigins.length > 0
                                ? token.allowedOrigins.join(', ')
                                : 'Any origin'}
                            </div>
                          )}
                        </TableCell>
                        <TableCell className="whitespace-nowrap font-mono text-xs text-muted-foreground">
                          {publishable && token.publishableKey ? (
                            <button
                              type="button"
                              className="inline-flex items-center gap-1 hover:text-foreground"
                              onClick={() => copy(token.publishableKey!)}
                              title="Copy the key"
                            >
                              {token.hint}
                              <CopyIcon size={12} />
                            </button>
                          ) : (
                            token.hint
                          )}
                        </TableCell>
                        <TableCell className="w-px whitespace-nowrap text-sm text-muted-foreground">
                          {parseAndFormatDateTime(token.createdAt)}
                        </TableCell>
                        <TableCell className="w-px whitespace-nowrap text-sm text-muted-foreground">
                          {token.lastUsedAt ? parseAndFormatDateTime(token.lastUsedAt) : 'Never'}
                        </TableCell>
                        <TableCell className="w-px whitespace-nowrap">
                          {rollable && (
                            <Button
                              variant="ghost"
                              size="sm"
                              title="Roll the key"
                              onClick={() => {
                                setRollGrace('24')
                                setTokenToRoll({ id: token.id, name: token.name })
                              }}
                              className="h-8 w-8 p-0 text-muted-foreground hover:text-foreground"
                            >
                              <RefreshCwIcon size={16} />
                            </Button>
                          )}
                          <Button
                            variant="ghost"
                            size="sm"
                            title="Revoke the key"
                            onClick={() => setTokenToDelete({ id: token.id, name: token.name })}
                            className="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
                          >
                            <Trash2Icon size={16} />
                          </Button>
                        </TableCell>
                      </TableRow>
                    )
                  })}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      </SettingsSection>

      <Dialog
        open={isCreateDialogOpen}
        onOpenChange={open => (open ? setIsCreateDialogOpen(true) : resetForm())}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create API key</DialogTitle>
            <DialogDescription>Name it to tell it apart later.</DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <FieldLabel htmlFor="token-name">Name</FieldLabel>
              <Input
                id="token-name"
                placeholder="e.g., Production server"
                value={tokenName}
                onChange={e => setTokenName(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter') {
                    createApiToken()
                  }
                }}
              />
            </div>
            <RadioGroup
              value={kind}
              onValueChange={value => setKind(value as ApiKeyKind)}
              className="gap-2"
            >
              {KINDS.map(option => {
                const id = `api-key-kind-${option.value}`
                return (
                  <Label
                    key={option.value}
                    htmlFor={id}
                    className={cn(
                      'flex cursor-pointer items-start gap-2.5 rounded-lg border border-border bg-card p-3 font-normal',
                      kind === option.value && 'border-primary'
                    )}
                  >
                    <RadioGroupItem value={option.value} id={id} className="mt-0.5" />
                    <span className="space-y-0.5">
                      <span className="block text-[13px] font-medium">{option.label}</span>
                      <span className="block text-xs leading-snug text-muted-foreground">
                        {option.description}
                      </span>
                    </span>
                  </Label>
                )
              })}
            </RadioGroup>
            {kind === 'secret' && (
              <div className="space-y-3">
                <div className="space-y-2">
                  <FieldLabel>Access</FieldLabel>
                  <RadioGroup
                    value={restricted ? 'restricted' : 'full'}
                    onValueChange={value => setRestricted(value === 'restricted')}
                    className="flex gap-4"
                  >
                    {[
                      { value: 'full', label: 'Full access' },
                      { value: 'restricted', label: 'Restricted' },
                    ].map(option => (
                      <Label
                        key={option.value}
                        htmlFor={`api-key-access-${option.value}`}
                        className="flex cursor-pointer items-center gap-2 font-normal"
                      >
                        <RadioGroupItem
                          value={option.value}
                          id={`api-key-access-${option.value}`}
                        />
                        {option.label}
                      </Label>
                    ))}
                  </RadioGroup>
                </div>
                {restricted && (
                  <div className="divide-y divide-border rounded-lg border border-border">
                    {RESOURCES.map(resource => (
                      <div
                        key={resource.key}
                        className="flex items-center justify-between gap-3 px-3 py-2"
                      >
                        <span className="min-w-0">
                          <span className="block text-[13px] font-medium">{resource.label}</span>
                          <span className="block truncate text-xs text-muted-foreground">
                            {resource.description}
                          </span>
                        </span>
                        <Select
                          value={levels[resource.key]}
                          onValueChange={value =>
                            setLevels({ ...levels, [resource.key]: value as AccessLevel })
                          }
                        >
                          <SelectTrigger className="h-8 w-24 shrink-0">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="none">None</SelectItem>
                            <SelectItem value="read">Read</SelectItem>
                            <SelectItem value="write">Write</SelectItem>
                          </SelectContent>
                        </Select>
                      </div>
                    ))}
                  </div>
                )}
                {grantsNothing && (
                  <p className="text-xs text-destructive">Grant at least one resource.</p>
                )}
                <div className="space-y-2">
                  <FieldLabel>Expires</FieldLabel>
                  <Select value={lifetime} onValueChange={setLifetime}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {LIFETIMES.map(option => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {lifetime !== 'never' && (
                    <p className="text-xs text-muted-foreground">
                      Roll the key before it expires: the new key gets the same lifetime.
                    </p>
                  )}
                </div>
              </div>
            )}
            {kind === 'publishable' && (
              <Collapsible open={originsOpen} onOpenChange={setOriginsOpen} className="space-y-2">
                <CollapsibleTrigger asChild>
                  <button
                    type="button"
                    className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground"
                  >
                    <ChevronRightIcon
                      size={12}
                      className={cn('transition-transform', originsOpen && 'rotate-90')}
                    />
                    Restrict to your websites
                  </button>
                </CollapsibleTrigger>
                <CollapsibleContent className="space-y-1.5">
                  <Textarea
                    id="token-origins"
                    placeholder={'https://www.example.com\nhttp://localhost:3000'}
                    value={originsText}
                    onChange={e => setOriginsText(e.target.value)}
                    rows={3}
                  />
                  <p
                    className={cn(
                      'text-xs',
                      refused.length > 0 ? 'text-destructive' : 'text-muted-foreground'
                    )}
                  >
                    {refused.length > 0
                      ? `Not an origin: ${refused.join(', ')}. Use scheme://host[:port].`
                      : 'One origin per line. Browsers on other sites are refused; empty accepts any.'}
                  </p>
                </CollapsibleContent>
              </Collapsible>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={resetForm}>
              Cancel
            </Button>
            <Button
              onClick={createApiToken}
              disabled={!tokenName.trim() || loading || refused.length > 0 || grantsNothing}
            >
              {loading ? 'Creating...' : 'Create key'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog open={!!tokenToRoll} onOpenChange={() => setTokenToRoll(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Roll API key?</AlertDialogTitle>
            <AlertDialogDescription>
              A new secret replaces <span className="font-semibold">{tokenToRoll?.name}</span>, with
              the same permissions and lifetime. Keep the old one working while you deploy the new
              one.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="space-y-2">
            <FieldLabel>The old key stops working</FieldLabel>
            <Select value={rollGrace} onValueChange={setRollGrace}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {ROLL_GRACE.map(option => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              Event ingestion may keep accepting the old key for up to two minutes.
            </p>
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={rollTokenMut.isPending}
              onClick={() =>
                tokenToRoll &&
                rollTokenMut.mutate({
                  id: tokenToRoll.id,
                  gracePeriodHours: ROLL_GRACE.find(g => g.value === rollGrace)?.hours ?? 0,
                })
              }
            >
              Roll key
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={!!tokenToDelete} onOpenChange={() => setTokenToDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Revoke API key?</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to revoke{' '}
              <span className="font-semibold">{tokenToDelete?.name}</span>? This action cannot be
              undone and anything using this key will stop working within two minutes.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleDeleteToken} variant="destructive">
              Revoke key
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
