import { useQuery } from '@tanstack/react-query'
import { createContext, useContext } from 'react'

import PageLoader from '@/components/atoms/PageLoader/PageLoader'
import { env } from '@/lib/env'

interface FlagProviderProps {
  children?: React.ReactNode
}

interface FlagData {
  orySdkUrl: string
}

export const FlagContext = createContext<{
  flags: FlagData | null
}>({ flags: null })

const FLAGS_API_URL = `${env.apiUrl}/.flags`

async function fetchConfigFromBackend() {
  try {
    const response = await fetch(FLAGS_API_URL)
    const config = await response.json()
    return config as FlagData
  } catch (error) {
    console.error('Error fetching configuration from backend:', error)
    return null
  }
}

export const FlagsProvider: React.FC<FlagProviderProps> = ({ children }) => {
  const query = useQuery({
    queryKey: ['todos'],
    queryFn: fetchConfigFromBackend,
  })
  // TODO handle errors

  return query.data ? (
    <FlagContext.Provider value={{ flags: query.data }}>{children}</FlagContext.Provider>
  ) : (
    <PageLoader />
  )
}

export const useFlags = () => {
  const { flags } = useContext(FlagContext)

  if (!flags) throw new Error('Flags context was not initialized')

  return flags
}
