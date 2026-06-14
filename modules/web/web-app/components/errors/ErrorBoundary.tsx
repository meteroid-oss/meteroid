import { Button } from '@md/ui'
import { AlertTriangle } from 'lucide-react'
import { Component, ErrorInfo, ReactNode } from 'react'

type FallbackRender = (props: { error: Error; reset: () => void }) => ReactNode

interface Props {
  children: ReactNode
  /** Custom fallback. A render function receives the error and a reset callback. */
  fallback?: ReactNode | FallbackRender
  /** When this value changes while an error is shown, the boundary resets. */
  resetKey?: unknown
  onError?: (error: Error, info: ErrorInfo) => void
}

interface State {
  error: Error | null
}

// Generic component-level error boundary. Use it to isolate independently
// fetching widgets (cards, charts, panels) so one failing piece degrades to a
// small fallback instead of blanking the whole page.
//
// `resetKey` (e.g. the current pathname or an entity id) clears the error when
// it changes, without remounting children on every render.
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('ErrorBoundary caught an error', error, info)
    this.props.onError?.(error, info)
  }

  componentDidUpdate(prev: Props) {
    if (this.state.error && prev.resetKey !== this.props.resetKey) {
      this.reset()
    }
  }

  reset = () => this.setState({ error: null })

  render() {
    const { error } = this.state
    if (!error) return this.props.children

    const { fallback } = this.props
    if (typeof fallback === 'function') return (fallback as FallbackRender)({ error, reset: this.reset })
    if (fallback !== undefined) return fallback

    return <DefaultFallback error={error} reset={this.reset} />
  }
}

const DefaultFallback = ({ error, reset }: { error: Error; reset: () => void }) => (
  <div className="flex flex-col items-center justify-center gap-3 rounded-lg border border-border bg-card p-6 text-center">
    <AlertTriangle className="text-warning" size={20} />
    <div className="text-sm font-medium text-foreground">Something went wrong</div>
    <div className="max-w-md text-[13px] text-muted-foreground">{error.message}</div>
    <Button size="sm" variant="secondary" onClick={reset}>
      Try again
    </Button>
  </div>
)
