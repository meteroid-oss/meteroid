import { motion, Variants } from 'framer-motion'
import * as React from 'react'

import { useWizard } from '../Wizard'

const variants: Variants = {
  enter: (direction: number) => {
    return {
      x: direction > 0 ? 800 : -800,
      opacity: 0,
    }
  },
  center: {
    zIndex: 1,
    x: 0,
    opacity: 1,
  },
  exit: (direction: number) => {
    return {
      zIndex: 0,
      x: direction < 0 ? 800 : -800,
      opacity: 0,
      transition: { duration: 0.2 },
    }
  },
}

type Props = {
  previousStep: () => void
  children: React.ReactNode
}

const AnimatedStep: React.FC<Props> = React.memo(
  ({ children, previousStep: previousStepIndex }) => {
    const { activeStep } = useWizard()

    React.useEffect(() => {
      return () => {
        // eslint-disable-next-line @typescript-eslint/ban-ts-comment
        // @ts-ignore
        previousStepIndex.current = activeStep
      }
    }, [activeStep, previousStepIndex])

    return (
      <motion.div
        // eslint-disable-next-line @typescript-eslint/ban-ts-comment
        // @ts-ignore
        custom={activeStep - previousStepIndex.current}
        variants={variants}
        initial="enter"
        animate="center"
        exit="exit"
        transition={{
          type: 'spring',
          stiffness: 300,
          damping: 30,
        }}
      >
        {children}
      </motion.div>
    )
  }
)

export default AnimatedStep
