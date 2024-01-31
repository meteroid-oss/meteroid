import { useContext } from 'react'

import { FlagContext } from 'providers/FlagsProvider'

export const useFlags = () => {
  const { flags } = useContext(FlagContext)

  if (!flags) throw new Error('Flags context was not initialized')

  return flags
}
