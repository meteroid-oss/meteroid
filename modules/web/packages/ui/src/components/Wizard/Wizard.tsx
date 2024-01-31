import { AnimatePresence } from 'framer-motion'
import * as ReactWizard from 'react-use-wizard'

import AnimatedStep from './components/AnimatedStep'

import type { ReactNode } from 'react'

type ReactWizardProps = typeof ReactWizard.Wizard
type WizardProps = Pick<ReactWizardProps, keyof ReactWizardProps> & {
  header?: ReactNode
  children?: ReactNode
}

function Wizard(props: WizardProps) {
  return <ReactWizard.Wizard wrapper={<AnimatePresence mode="wait" />} {...props} />
}

Wizard.displayName = 'Wizard'
Wizard.AnimatedStep = AnimatedStep

export const useWizard = ReactWizard.useWizard
export { Wizard }
