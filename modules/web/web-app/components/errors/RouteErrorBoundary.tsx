import { ConnectError } from '@connectrpc/connect'
import { Button } from '@md/ui'
import { AlertTriangle } from 'lucide-react'
import { isRouteErrorResponse, useNavigate, useRouteError } from 'react-router-dom'

// `errorElement` for the data router. Catches render and loader errors thrown
// anywhere in the subtree below the route it is attached to, so a thrown error
// shows a recoverable screen instead of a blank white page.
export const RouteErrorBoundary = () => {
  const error = useRouteError()
  const navigate = useNavigate()

  const { title, message } = describeError(error)

  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-4 p-8 text-center">
      <AlertTriangle className="text-warning" size={28} />
      <div className="text-lg font-medium text-foreground">{title}</div>
      <div className="max-w-md text-sm text-muted-foreground">{message}</div>
      <div className="flex gap-2">
        <Button variant="secondary" size="sm" onClick={() => navigate(-1)}>
          Go back
        </Button>
        <Button size="sm" onClick={() => window.location.reload()}>
          Reload
        </Button>
      </div>
    </div>
  )
}

const describeError = (error: unknown): { title: string; message: string } => {
  if (isRouteErrorResponse(error)) {
    return {
      title: `${error.status} ${error.statusText}`,
      message: typeof error.data === 'string' ? error.data : 'This page could not be loaded.',
    }
  }

  // connect-query/grpc errors surface here when thrown from a loader.
  if (error instanceof ConnectError) {
    return { title: 'Request failed', message: error.rawMessage }
  }

  if (error instanceof Error) {
    return { title: 'Something went wrong', message: error.message }
  }

  return { title: 'Something went wrong', message: 'An unexpected error occurred.' }
}
