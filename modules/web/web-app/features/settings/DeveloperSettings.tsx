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
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CheckIcon, CopyIcon, PlusIcon, Trash2Icon } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { toast } from 'sonner'

import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
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
export const DeveloperSettings: FunctionComponent = () => {
  const queryClient = useQueryClient()

  const [displayed, setDisplayed] = useState<ApiToken>()
  const [loading, setLoading] = useState(false)
  const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false)
  const [tokenName, setTokenName] = useState('')
  const [tokenToDelete, setTokenToDelete] = useState<{ id: string; name: string } | null>(null)

  const createTokenMut = useMutation(createApiTokenMutation, {
    onSuccess: async res => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listApiTokens) })
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
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listApiTokens) })
      toast.success('API token revoked successfully')
      setTokenToDelete(null)
    },
    onError: () => {
      toast.error('Failed to revoke API token')
    },
  })

  const tokens = useQuery(listApiTokens)

  // Sort tokens by creation date (newest first)
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
    <>
      <div className="space-y-6 w-full border-t ">
        <Tabs defaultValue="api-keys" className="w-full">
          <TabsList className="w-full justify-start">
            <TabsTrigger value="api-keys">Api keys</TabsTrigger>
            <TabsTrigger value="api-docs">API Documentation</TabsTrigger>
            <TabsTrigger value="webhooks">Webhooks</TabsTrigger>
          </TabsList>
          <TabsContent value="api-keys">
            <div className="flex justify-between py-4">
              <div>
                <h1 className="text-lg pb-4 font-semibold">Api keys</h1>
                <p className="text-sm text-muted-foreground">
                  Create an API key to access our API.
                </p>
              </div>
              <Button hasIcon onClick={() => setIsCreateDialogOpen(true)} size="sm">
                <PlusIcon size={12} /> Create api token
              </Button>
            </div>
            <div className="space-y-4">
              {loading && (
                <Alert variant="default" className="max-w-2xl">
                  <Skeleton height={20} width="100%" />
                </Alert>
              )}
              {!loading && displayed && (
                <Alert variant="success" className="max-w-2xl">
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
                      onClick={() =>
                        copyToClipboard(displayed.apiKey, () => toast.success('Copied !'))
                      }
                    />
                  </AlertDescription>
                </Alert>
              )}

              <div className="max-w-4xl">
                {sortedTokens.length === 0 ? (
                  <div className="border border-border rounded-lg p-8 text-center">
                    <p className="text-sm text-muted-foreground">No API keys yet</p>
                  </div>
                ) : (
                  <Table containerClassName="border border-border rounded-lg">
                    <TableHeader>
                      <TableRow>
                        <TableHead className="w-[300px]">Name</TableHead>
                        <TableHead className="w-[200px]">Hint</TableHead>
                        <TableHead>Created</TableHead>
                        <TableHead className="w-[80px]"></TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {sortedTokens.map(token => (
                        <TableRow key={token.id}>
                          <TableCell className="font-medium">{token.name}</TableCell>
                          <TableCell className="font-mono text-xs text-muted-foreground">
                            {token.hint}
                          </TableCell>
                          <TableCell className="text-sm text-muted-foreground">
                            {parseAndFormatDateTime(token.createdAt)}
                          </TableCell>
                          <TableCell>
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
                )}
              </div>
            </div>
          </TabsContent>
          <TabsContent value="api-docs">
            <div className="py-4">
              <h1 className="text-lg pb-2 font-semibold">API Documentation</h1>

              <div
                className="border border-border rounded-lg overflow-hidden"
                style={{ height: 'calc(100vh - 300px)', minHeight: '600px' }}
              >
                <iframe
                  src={`${env.meteroidRestApiUri}/scalar`}
                  className="w-full h-full"
                  title="API Documentation"
                  style={{ border: 'none' }}
                />
              </div>
            </div>
          </TabsContent>
          <TabsContent value="webhooks">Not implemented</TabsContent>
        </Tabs>
      </div>

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
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Revoke Token
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
