import { useQuery } from '@connectrpc/connect-query'
import { Flex } from '@ui/components'
import { Outlet, useLocation } from 'react-router-dom'
import { match } from 'ts-pattern'

import { MeteroidTitle } from '@/components/svg'
import { useLogout } from '@/hooks/useLogout'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'
import { useForceTheme } from 'providers/ThemeProvider'

export const OnboardingLayout = () => {
  useForceTheme('dark')

  const meQuery = useQuery(me)

  const logout = useLogout()
  const { pathname } = useLocation()

  const email = meQuery.data?.user?.email

  const currentStep = match(pathname)
    .with('/onboarding/user', () => 0)
    .with('/onboarding/organization', () => 1)
    .otherwise(() => 0)

  return (
    <div
      className="dark min-h-screen flex flex-col overflow-hidden relative"
      style={{
        background: 'linear-gradient(0deg, #000 0%, #000 100%), #111',
      }}
    >
      <Flex direction="column" className="p-6 h-full w-full">
        <Flex justify="between" align="center">
          <MeteroidTitle forceTheme="dark" />
          <div className="text-xs">
            <span className="text-muted-foreground mr-1">Logged in as {email}</span>
            <span className="underline cursor-pointer" onClick={() => logout()}>
              Log out
            </span>
          </div>
        </Flex>
        <Flex
          justify="center"
          align="center"
          className="px-2 xl:px-12 2xl:px-44 py-24 w-full flex-grow max-w-[2200px] mx-auto"
        >
          <Flex className="w-full h-full">
            <Outlet />
          </Flex>
        </Flex>
        <Flex justify="center">
          {[0, 1].map(step => (
            <div
              key={step}
              className={`w-2 h-2 mx-1 rounded-full ${
                step === currentStep ? 'bg-[#76777D]' : 'bg-[#232323]'
              }`}
            />
          ))}
        </Flex>
      </Flex>
    </div>
  )
}
