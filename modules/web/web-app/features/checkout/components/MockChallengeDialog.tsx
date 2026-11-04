import { ShieldCheck } from 'lucide-react'
import { useEffect } from 'react'
import { createRoot } from 'react-dom/client'

interface MockChallengeDialogProps {
  onApprove: () => void
  onFail: () => void
}

/** Stand-in for a bank's 3-D Secure prompt, themed with the portal tokens. Escape fails it, as
 *  abandoning a real challenge does. */
const MockChallengeDialog = ({ onApprove, onFail }: MockChallengeDialogProps) => {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onFail()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onFail])

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="mock-challenge-title"
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ background: 'var(--mtp-overlay)' }}
    >
      <div
        className="w-full max-w-sm p-6 shadow-xl"
        style={{
          background: 'var(--mtp-surface)',
          color: 'var(--mtp-text)',
          border: '1px solid var(--mtp-border)',
          borderRadius: 'var(--mtp-r-card)',
        }}
      >
        <div className="flex items-center gap-2 mb-2">
          <ShieldCheck size={20} style={{ color: 'var(--mtp-accent)' }} />
          <h2 id="mock-challenge-title" className="font-medium">
            Bank authentication
          </h2>
        </div>
        <p className="text-sm mb-5" style={{ color: 'var(--mtp-text-2)' }}>
          Test gateway stand-in for 3-D Secure. A real bank would ask you to confirm this payment in
          its app; choose how that goes.
        </p>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={onFail}
            className="flex-1 py-2.5 text-sm border hover:opacity-90"
            style={{ borderColor: 'var(--mtp-border)', borderRadius: 'var(--mtp-r-ctrl)' }}
          >
            Fail authentication
          </button>
          <button
            type="button"
            autoFocus
            onClick={onApprove}
            className="flex-1 py-2.5 text-sm font-medium hover:opacity-90"
            style={{
              background: 'var(--mtp-accent)',
              color: 'var(--mtp-on-accent)',
              borderRadius: 'var(--mtp-r-ctrl)',
            }}
          >
            Approve
          </button>
        </div>
      </div>
    </div>
  )
}

/** Mounts the dialog inside the portal theme root (so it inherits the tokens) and resolves with
 *  the customer's choice. Imperative because `completeNextAction` runs outside any component. */
export const askMockChallenge = (): Promise<boolean> =>
  new Promise(resolve => {
    const host = document.querySelector('[data-mtp-theme]') ?? document.body
    const container = document.createElement('div')
    host.appendChild(container)
    const root = createRoot(container)
    const finish = (approve: boolean) => {
      root.unmount()
      container.remove()
      resolve(approve)
    }
    root.render(<MockChallengeDialog onApprove={() => finish(true)} onFail={() => finish(false)} />)
  })
