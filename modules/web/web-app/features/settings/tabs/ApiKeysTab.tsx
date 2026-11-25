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
  Input,
  InputWithIcon,
  Label,
  RadioGroup,
  RadioGroupItem,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Textarea,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CheckIcon, ChevronRightIcon, CopyIcon, PlusIcon, Trash2Icon } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { toast } from 'sonner'

import { ApiKeyKind, invalidOrigins, parseOrigins } from '@/features/settings/apiKeys/apiKeyForm'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import {
  createApiToken as createApiTokenMutation,
  createPublishableKey as createPublishableKeyMutation,
  listApiTokens,
  revokeApiToken as revokeApiTokenMutation,
} from '@/rpc/api/apitokens/v1/apitokens-ApiTokensService_connectquery'
import { ApiTokenKind } from '@/rpc/api/apitokens/v1/models_pb'
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

  const refused = kind === 'publishable' ? invalidOrigins(originsText) : []

  const resetForm = () => {
    setIsCreateDialogOpen(false)
    setTokenName('')
    setKind('secret')
    setOriginsOpen(false)
    setOriginsText('')
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
    if (refused.length > 0) return
    setLoading(true)
    if (kind === 'publishable') {
      createPublishableMut.mutateAsync({
        name: tokenName.trim(),
        allowedOrigins: parseOrigins(originsText),
      })
    } else {
      createTokenMut.mutateAsync({
        name: tokenName.trim(),
      })
    }
  }

  const handleDeleteToken = async () => {
    if (!tokenToDelete) return

    revokeTokenMut.mutateAsync({
      id: tokenToDelete.id,
    })
  }

  const copy = (value: string) => copyToClipboard(value, () => toast.success('Copied !'))

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <SettingsSection
        divided={false}
        title="API keys"
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
            <div className="rounded-lg border border-border p-8 text-center">
              <p className="text-sm text-muted-foreground">No API keys yet</p>
            </div>
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
                          </div>
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
                        <TableCell className="w-px">
                          <Button
                            variant="ghost"
                            size="sm"
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
              <Label htmlFor="token-name">Name</Label>
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
              disabled={!tokenName.trim() || loading || refused.length > 0}
            >
              {loading ? 'Creating...' : 'Create key'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

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
