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
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  InputWithIcon,
  Label,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CheckIcon, CopyIcon, PlusIcon, Trash2Icon } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { toast } from 'sonner'

import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import {
  createApiToken as createApiTokenMutation,
  listApiTokens,
  revokeApiToken as revokeApiTokenMutation,
} from '@/rpc/api/apitokens/v1/apitokens-ApiTokensService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'

interface ApiToken {
  id: string
  name: string
  apiKey: string
}

export const ApiKeysTab: FunctionComponent = () => {
  const queryClient = useQueryClient()

  const [displayed, setDisplayed] = useState<ApiToken>()
  const [loading, setLoading] = useState(false)
  const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false)
  const [tokenName, setTokenName] = useState('')
  const [tokenToDelete, setTokenToDelete] = useState<{ id: string; name: string } | null>(null)

  const createTokenMut = useMutation(createApiTokenMutation, {
    onSuccess: async res => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listApiTokens,
          cardinality: 'finite',
        }),
      })
      setDisplayed({
        id: res.details!.id,
        name: res.details!.name,
        apiKey: res.apiKey,
      })
      setIsCreateDialogOpen(false)
      setTokenName('')
    },
    onSettled() {
      setTimeout(() => {
        setLoading(false)
      }, 500)
    },
  })

  const revokeTokenMut = useMutation(revokeApiTokenMutation, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listApiTokens,
          cardinality: 'finite',
        }),
      })
      toast.success('API token revoked successfully')
      setTokenToDelete(null)
    },
    onError: () => {
      toast.error('Failed to revoke API token')
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
      toast.error('Please enter a token name')
      return
    }
    setLoading(true)
    createTokenMut.mutateAsync({
      name: tokenName.trim(),
    })
  }

  const handleDeleteToken = async () => {
    if (!tokenToDelete) return

    revokeTokenMut.mutateAsync({
      id: tokenToDelete.id,
    })
  }

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <SettingsSection
        divided={false}
        title="API keys"
        description="Create an API key to access our API."
        action={
          <Button hasIcon onClick={() => setIsCreateDialogOpen(true)} size="sm">
            <PlusIcon size={12} /> Create api token
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
                  This is your api key. Copy it and store it securely, it will not be displayed
                  again
                </div>
                <InputWithIcon
                  value={displayed.apiKey}
                  readOnly
                  icon={<CopyIcon className="group-hover:text-success" />}
                  className="cursor-pointer"
                  containerClassName="group"
                  onClick={() => copyToClipboard(displayed.apiKey, () => toast.success('Copied !'))}
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
                    <TableHead className="whitespace-nowrap">Hint</TableHead>
                    <TableHead className="w-px whitespace-nowrap">Created</TableHead>
                    <TableHead className="w-px whitespace-nowrap">Last used</TableHead>
                    <TableHead className="w-px"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {sortedTokens.map(token => (
                    <TableRow key={token.id}>
                      <TableCell className="max-w-[240px] truncate font-medium" title={token.name}>
                        {token.name}
                      </TableCell>
                      <TableCell className="whitespace-nowrap font-mono text-xs text-muted-foreground">
                        {token.hint}
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
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      </SettingsSection>

      <Dialog open={isCreateDialogOpen} onOpenChange={setIsCreateDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create API Token</DialogTitle>
            <DialogDescription>
              Enter a name for your API token. This will help you identify it later.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label htmlFor="token-name">Token Name</Label>
              <Input
                id="token-name"
                placeholder="e.g., Production API Key"
                value={tokenName}
                onChange={e => setTokenName(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter') {
                    createApiToken()
                  }
                }}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setIsCreateDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={createApiToken} disabled={!tokenName.trim() || loading}>
              {loading ? 'Creating...' : 'Create Token'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog open={!!tokenToDelete} onOpenChange={() => setTokenToDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Revoke API Token?</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to revoke{' '}
              <span className="font-semibold">{tokenToDelete?.name}</span>? This action cannot be
              undone and any applications using this token will stop working immediately.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDeleteToken}
              variant="destructive"
            >
              Revoke Token
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
