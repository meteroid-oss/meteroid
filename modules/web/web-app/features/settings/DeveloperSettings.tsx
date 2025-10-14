import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  AlertDescription,
  AlertTitle,
  Button,
  InputWithIcon,
  Skeleton,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CheckIcon, CopyIcon, PlusIcon } from 'lucide-react'
import { nanoid } from 'nanoid'
import { FunctionComponent, useState } from 'react'
import { toast } from 'sonner'

import { SimpleTable } from '@/components/table/SimpleTable'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import {
  createApiToken as createApiTokenMutation,
  listApiTokens,
} from '@/rpc/api/apitokens/v1/apitokens-ApiTokensService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

interface ApiToken {
  id: string
  name: string
  apiKey: string
}
export const DeveloperSettings: FunctionComponent = () => {
  const queryClient = useQueryClient()

  const [displayed, setDisplayed] = useState<ApiToken>()
  const [loading, setLoading] = useState(false)

  const createTokenMut = useMutation(createApiTokenMutation, {
    onSuccess: async res => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listApiTokens) })
      setDisplayed({
        id: res.details!.id,
        name: res.details!.name,
        apiKey: res.apiKey,
      })
    },
    onSettled() {
      setTimeout(() => {
        setLoading(false)
      }, 500)
    },
  })

  const tokens = useQuery(listApiTokens)

  const createApiToken = async () => {
    setLoading(true)
    createTokenMut.mutateAsync({
      name: `token-${nanoid(3)}`,
    })
  }

  return (
    <>
      <div className="space-y-6 w-full border-t ">
        <Tabs defaultValue="api-keys" className="w-full">
          <TabsList className="w-full justify-start">
            <TabsTrigger value="api-keys">Api keys</TabsTrigger>
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
              <Button hasIcon onClick={() => createApiToken()} size="sm">
                <PlusIcon size={12} /> Create api token
              </Button>
            </div>
            <div className="space-y-4 max-w-xl">
              {loading && (
                <>
                  <Alert variant="default">
                    <Skeleton height={20} width="100%" />
                  </Alert>
                </>
              )}
              {!loading && displayed && (
                <div>
                  <Alert variant="success">
                    <CheckIcon size={16} />
                    <AlertTitle className="pb-2 pt-1">Success !</AlertTitle>
                    <AlertDescription className="text-foreground">
                      <div>
                        This is your api key. Copy it and store it securely, it will not be
                        displayed again
                      </div>
                      <div>
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
                      </div>
                    </AlertDescription>
                  </Alert>
                </div>
              )}

              <ul className="space-y-2">
                {tokens.data?.apiTokens?.length === 0 && (
                  <SimpleTable
                    columns={[]}
                    data={[]}
                    emptyMessage="No Api Key"
                    containerClassName="max-w-xl max-h-xl"
                  />
                )}
                {tokens.data?.apiTokens?.map(token => (
                  <li key={token.id} className="w-xl border border-border rounded-xl p-4">
                    <h3 className="font-semibold">{token.name}</h3>
                    <div className="text-sm font-semibold">Hint: {token.hint}</div>
                    <div className="text-sm text-muted-foreground">
                      Created on: {parseAndFormatDate(token.createdAt)}
                    </div>
                  </li>
                ))}
              </ul>
            </div>
          </TabsContent>
          <TabsContent value="webhooks">Not implemented</TabsContent>
        </Tabs>
      </div>
    </>
  )
}
