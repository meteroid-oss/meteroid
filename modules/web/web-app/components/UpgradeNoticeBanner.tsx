import { TriangleAlert, X } from 'lucide-react'
import { useState } from 'react'

const DISMISSED_KEY = 'meteroid:notice:next-major-manual-migration'
const LEARN_MORE_URL = 'https://github.com/meteroid-oss/meteroid#upcoming-breaking-release'

const isDismissed = () => {
  try {
    return localStorage.getItem(DISMISSED_KEY) === '1'
  } catch {
    return false
  }
}

export const UpgradeNoticeBanner = () => {
  const [dismissed, setDismissed] = useState(isDismissed)

  if (dismissed) return null

  const dismiss = () => {
    try {
      localStorage.setItem(DISMISSED_KEY, '1')
    } catch {
      // storage unavailable: hide for this session only
    }
    setDismissed(true)
  }

  return (
    <div
      role="status"
      className="flex items-start gap-3 border-b border-warning/40 bg-warning/10 px-6 py-2 text-sm text-foreground"
    >
      <TriangleAlert size={16} className="mt-0.5 shrink-0 text-warning" />
      <p className="flex-1">
        <span className="font-medium">The next major release of Meteroid is a new baseline.</span>{' '}
        It will not upgrade in place from this version and will require a manual data migration.
        Hold off on upgrading until you have read the migration notes.{' '}
        <a
          href={LEARN_MORE_URL}
          target="_blank"
          rel="noreferrer"
          className="font-medium underline underline-offset-2"
        >
          Learn more
        </a>
      </p>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Dismiss"
        className="shrink-0 rounded p-0.5 text-muted-foreground hover:bg-warning/20 hover:text-foreground"
      >
        <X size={16} />
      </button>
    </div>
  )
}
