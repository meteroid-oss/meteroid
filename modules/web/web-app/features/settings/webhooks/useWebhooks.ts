import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'

import { useQuery } from '@/lib/connectrpc'
import {
  archiveWebhookEndpoint,
  createWebhookEndpoint,
  listWebhookEndpoints,
  resendWebhookDelivery,
  resumeWebhookEndpoint,
  rotateWebhookEndpointSecret,
  sendTestWebhook,
  updateWebhookEndpoint,
} from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'
import { WebhooksService } from '@/rpc/api/webhooksout/v1/webhooksout_pb'

/**
 * Endpoint list + every webhook mutation, each invalidating the whole WebhooksService
 * cache so endpoints, deliveries, stats and the revealed secret all stay in sync.
 */
export const useWebhooks = () => {
  const queryClient = useQueryClient()
  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: WebhooksService, cardinality: undefined }),
    })

  const endpointsQuery = useQuery(listWebhookEndpoints)

  return {
    endpoints: endpointsQuery.data?.endpoints ?? [],
    isLoading: endpointsQuery.isLoading,
    create: useMutation(createWebhookEndpoint, { onSuccess: invalidate }),
    update: useMutation(updateWebhookEndpoint, { onSuccess: invalidate }),
    archive: useMutation(archiveWebhookEndpoint, { onSuccess: invalidate }),
    resume: useMutation(resumeWebhookEndpoint, { onSuccess: invalidate }),
    rotateSecret: useMutation(rotateWebhookEndpointSecret, { onSuccess: invalidate }),
    sendTest: useMutation(sendTestWebhook, { onSuccess: invalidate }),
    resend: useMutation(resendWebhookDelivery, { onSuccess: invalidate }),
  }
}
