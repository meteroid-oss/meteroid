import { ReactNode } from 'react'

/** Footer of every connect-wizard step: the step's actions, right-aligned. */
export const WizardFooter = ({ children }: { children: ReactNode }) => (
  <div className="flex justify-end gap-2 border-t py-3">{children}</div>
)
