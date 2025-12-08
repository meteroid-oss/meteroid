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

export const App: React.FC = () => {
  const transport = createGrpcWebTransport({
    baseUrl: env.meteroidApiUri,
    interceptors: [errorInterceptor, loggingInterceptor, authInterceptor],
  })

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
