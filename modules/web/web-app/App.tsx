import { TransportProvider } from '@connectrpc/connect-query'
import { createGrpcWebTransport } from '@connectrpc/connect-web'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider } from 'react-router-dom'
import { Toaster } from 'sonner'

import {
  authInterceptor,
  errorInterceptor,
  loggingInterceptor,
} from '@/lib/connectrpc-interceptors'
import { env } from '@/lib/env'

import router from './router/router'

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 minutes
    },
  },
})

export const App: React.FC = () => {
  const transport = createGrpcWebTransport({
    baseUrl: env.meteroidApiUri,
    interceptors: [errorInterceptor, loggingInterceptor, authInterceptor],
  })
  return (
    <>
      <TransportProvider transport={transport}>
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
        </QueryClientProvider>
      </TransportProvider>

      <Toaster />
    </>
  )
}
