import { useQuery } from '@connectrpc/connect-query'
import { Button, Flex, Separator } from '@ui/components'
import { Outlet, useLocation, useSearchParams } from 'react-router-dom'

import { env } from "@/lib/env";
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

export const AuthFormLayout = () => {
  const location = useLocation()

  const getInstanceQuery = useQuery(getInstance)

  const isGoogleAuthEnabled = getInstanceQuery.data?.googleOauthClientId

  const isGithubAuthEnabled = false

  const isLogin = location.pathname === '/login'

  const [searchParams] = useSearchParams()

  const invite = searchParams.get('invite') ?? undefined

  const registrationClosed = getInstanceQuery.data && getInstanceQuery.data.instanceInitiated && !getInstanceQuery.data.multiOrganizationEnabled && !invite

  const title = isLogin ? 'Log in' : 'Sign up'

  const shouldShowOauth = (isGoogleAuthEnabled || isGithubAuthEnabled) && !(registrationClosed && !isLogin)


  return (
    <>
      <div className="font-medium text-xl -mb-0.5">{title}</div>
      <div className="text-muted-foreground text-[13px] mb-3 leading-[18px]">
        Automate your billing, create and test any pricing strategy, uncover growth
        opportunities.
      </div>
      {
        shouldShowOauth && (<>
            {isGoogleAuthEnabled && (
              <a href={`${env.meteroidRestApiUri}/oauth/google?is_signup=${!isLogin}&invite_key=${invite ?? ''}`}>
                <Button variant="default" size="md" className="w-full" hasIcon>
                  <img src="/company/google.svg" alt="Google" className="w-[19px] h-[19px] mb-0.5"/>
                  Continue with Google
                </Button>
              </a>
            )}
            {isGithubAuthEnabled && (
              <Button variant="secondary" size="md" className="w-full" hasIcon>
                <img src="/company/github.svg" alt="GitHub" className="w-[19px] h-[19px] mb-0.5"/>
                Continue with GitHub
              </Button>
            )}
          </>
        )
      }

      {
        shouldShowOauth && (<Flex align="center" justify="center" className="gap-2 w-full mt-1">
          <div className="flex-grow">
            <Separator/>
          </div>
          <div className="text-muted-foreground text-xs whitespace-nowrap">or</div>
          <div className="flex-grow">
            <Separator/>
          </div>
        </Flex>)
      }

      <Outlet/>
      {!isLogin && !registrationClosed && (
        <div className="text-[11px] text-center p-2 leading-4">
          <span className="text-muted-foreground ">By proceeding, you agree to our </span>
          <a href="https://meteroid.com/privacy" className="underline">
            Privacy Policy
          </a>
          <span className="text-muted-foreground mx-1">and</span>
          <a href="https://meteroid.com/terms" className="underline">
            Terms of service
          </a>
        </div>
      )}
    </>
  )
}
