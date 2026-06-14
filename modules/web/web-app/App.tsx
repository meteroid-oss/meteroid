import { TransportProvider } from '@connectrpc/connect-query'
import { createGrpcWebTransport } from '@connectrpc/connect-web'
import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider } from 'react-router-dom'
import { Toaster } from 'sonner'

import {
  authInterceptor,
  errorInterceptor,
  loggingInterceptor,
} from '@/lib/connectrpc-interceptors'
import { env } from '@/lib/env'
import { queryClient } from '@/lib/react-query'
import { useTheme } from 'providers/ThemeProvider'

import router from './router/router'

// A single, stable transport for the app's lifetime. It must not be recreated
// on render: connect-query derives the query-key's transport id from the
// transport reference, so a new instance would orphan the entire query cache.
const transport = createGrpcWebTransport({
  baseUrl: env.meteroidApiUri,
  interceptors: [errorInterceptor, loggingInterceptor, authInterceptor],
})

export const App: React.FC = () => {
  const theme = useTheme()

  return (
    <>
      <TransportProvider transport={transport}>
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
        </QueryClientProvider>
      </TransportProvider>

      <Toaster theme={theme.isDarkMode ? 'dark' : 'light'} />
    </>
  )
}
