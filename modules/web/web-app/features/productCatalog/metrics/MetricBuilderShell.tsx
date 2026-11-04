import { Button, Dialog, DialogContent, DialogTitle, Form } from '@md/ui'
import { Maximize2, Minimize2 } from 'lucide-react'
import { ReactNode, useRef, useState } from 'react'
import {
  Panel,
  PanelGroup,
  PanelResizeHandle,
  type ImperativePanelHandle,
} from 'react-resizable-panels'

interface MetricBuilderShellProps {
  title: string
  description?: ReactNode
  onClose: () => void
  /** Already-bound submit handler, e.g. `methods.handleSubmit(onSubmit)`. */
  onSubmit: (e: React.FormEvent) => void
  // The form's react-hook-form methods; the type varies between create and edit, so the
  // shell stays form-agnostic (fields address themselves through the FormProvider context).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  methods: any
  formSlot: ReactNode
  previewSlot: ReactNode
  footerSlot: ReactNode
}

/**
 * Settings-style modal shell for the metric builder: a large centered dialog with the form
 * on the left (capped to a readable width) and the live preview on the right, split by a
 * draggable divider. The expand button collapses the form so the preview fills the modal.
 */
export const MetricBuilderShell = ({
  title,
  description,
  onClose,
  onSubmit,
  methods,
  formSlot,
  previewSlot,
  footerSlot,
}: MetricBuilderShellProps) => {
  const formPanel = useRef<ImperativePanelHandle>(null)
  const [expanded, setExpanded] = useState(false)

  const toggleExpand = () => {
    const panel = formPanel.current
    if (!panel) return
    if (panel.isCollapsed()) {
      panel.expand()
      setExpanded(false)
    } else {
      panel.collapse()
      setExpanded(true)
    }
  }

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent
        aria-describedby={undefined}
        className="flex flex-col gap-0 overflow-hidden p-0 w-[calc(100vw-2rem)] max-w-[1400px] h-[calc(100dvh-3rem)] max-h-[920px] sm:rounded-xl"
      >
        <Form {...methods}>
          <form
            onSubmit={onSubmit}
            onKeyDown={e => {
              if (e.key === 'Enter' && e.target instanceof HTMLInputElement) e.preventDefault()
            }}
            className="flex h-full min-h-0 flex-col"
          >
            <header className="flex shrink-0 flex-col gap-0.5 border-b border-border px-6 py-4 pr-12">
              <DialogTitle className="text-base font-semibold">{title}</DialogTitle>
              {description && (
                <p className="text-[13px] leading-snug text-muted-foreground">{description}</p>
              )}
            </header>

            <PanelGroup direction="horizontal" className="min-h-0 flex-1">
              <Panel
                ref={formPanel}
                collapsible
                collapsedSize={0}
                defaultSize={52}
                minSize={34}
                className="min-w-0"
              >
                <div className="h-full overflow-y-auto px-6 py-5">
                  <div className="max-w-3xl space-y-6">{formSlot}</div>
                </div>
              </Panel>

              <PanelResizeHandle className="group relative flex w-2 shrink-0 items-center justify-center outline-none">
                <div className="h-full w-px bg-border transition-colors group-hover:bg-primary group-data-[resize-handle-state=drag]:bg-primary" />
                <div className="absolute h-8 w-1 rounded-full bg-transparent transition-colors group-hover:bg-primary/40" />
              </PanelResizeHandle>

              <Panel defaultSize={48} minSize={30} className="min-w-0">
                <div className="flex h-full flex-col px-6 py-5">
                  <div className="mb-3 flex items-center justify-between gap-3">
                    <h3 className="text-sm font-semibold tracking-tight">Preview</h3>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="h-7 w-7 shrink-0"
                      aria-label={expanded ? 'Restore form' : 'Expand preview'}
                      onClick={toggleExpand}
                    >
                      {expanded ? (
                        <Minimize2 className="h-4 w-4" />
                      ) : (
                        <Maximize2 className="h-4 w-4" />
                      )}
                    </Button>
                  </div>
                  <div className="min-h-0 flex-1 overflow-y-auto">{previewSlot}</div>
                </div>
              </Panel>
            </PanelGroup>

            <footer className="flex shrink-0 items-center justify-end gap-3 border-t border-border px-6 py-3">
              {footerSlot}
            </footer>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
