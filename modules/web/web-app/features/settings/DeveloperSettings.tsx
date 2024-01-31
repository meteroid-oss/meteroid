import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { Alert, ButtonAlt, Input, Tabs, TabsContent, TabsList, TabsTrigger } from '@ui/components'
import { nanoid } from 'nanoid'
import { FunctionComponent, useState } from 'react'

import { SimpleTable } from '@/components/table/SimpleTable'
import { useQuery } from '@/lib/connectrpc'
import {
  listApiTokens,
  createApiToken as createApiTokenMutation,
} from '@/rpc/api/apitokens/v1/apitokens-ApiTokensService_connectquery'

interface ApiToken {
  id: string
  name: string
  apiKey: string
}
export const DeveloperSettings: FunctionComponent = () => {
  const queryClient = useQueryClient()

  const [displayed, setDisplayed] = useState<ApiToken>()
  const createTokenMut = useMutation(createApiTokenMutation, {
    onSuccess: async res => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listApiTokens) })
      setDisplayed({
        id: res.details!.id,
        name: res.details!.name,
        apiKey: res.apiKey,
      })
    },
  })

  const tokens = useQuery(listApiTokens)

  const createApiToken = async () =>
    createTokenMut.mutateAsync({
      name: `token-${nanoid(3)}`,
    })

  return (
    <>
      <div className="p-6 space-y-6 w-full">
        <div className="space-y-2">
          <h3 className="">Developer Settings</h3>
          <div className="border-b border-slate-400" />
        </div>
        <Tabs defaultValue="api-keys" className="w-full">
          <TabsList className="w-full justify-start">
            <TabsTrigger value="api-keys">Api keys</TabsTrigger>
            <TabsTrigger value="webhooks">Webhooks</TabsTrigger>
            <TabsTrigger value="events">Event Debugger</TabsTrigger>
          </TabsList>
          <TabsContent value="api-keys">
            <div className="flex max-w-xl justify-end p-2">
              <ButtonAlt onClick={() => createApiToken()}>+ Create api token</ButtonAlt>
            </div>
            <div className="space-y-4 max-w-xl">
              {displayed && (
                <div>
                  <Alert variant="success">
                    <div>
                      <div>
                        This is your api key. Copy it and store it securely, it will not be
                        displayed again
                      </div>
                      <div>
                        <Input value={displayed.apiKey} readOnly copy />
                      </div>
                    </div>
                  </Alert>
                </div>
              )}

              <ul className="space-y-2">
                {tokens.data?.apiTokens?.length === 0 && (
                  <SimpleTable
                    columns={[]}
                    data={[]}
                    emptyMessage="No Api Key"
                    containerClassName="max-w-xl"
                  />
                )}
                {tokens.data?.apiTokens?.map(token => (
                  <li key={token.id} className="w-xl border border-slate-800 rounded-xl p-4">
                    <h3 className="font-semibold">{token.name}</h3>
                    <div className="text-sm font-semibold">Key: {token.hint}********</div>
                    <div className="text-sm">Created by: {token.createdBy} (todo resolve)</div>
                  </li>
                ))}
              </ul>
            </div>
          </TabsContent>
          <TabsContent value="webhooks">Not implemented</TabsContent>
          <TabsContent value="events">Not implemented</TabsContent>
        </Tabs>
      </div>
    </>
  )
}
