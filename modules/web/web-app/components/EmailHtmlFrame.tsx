import { cn } from '@md/ui'

interface EmailHtmlFrameProps {
  html: string
  title?: string
  className?: string
}

// Email bodies are untrusted HTML: the empty sandbox blocks scripts, forms and navigation.
export const EmailHtmlFrame = ({
  html,
  title = 'email preview',
  className,
}: EmailHtmlFrameProps) => (
  <iframe
    title={title}
    srcDoc={html}
    sandbox=""
    className={cn('w-full h-[60vh] border rounded bg-background', className)}
  />
)
