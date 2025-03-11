import { Button, Flex, Separator } from '@ui/components'
import { Link, Outlet, useLocation } from 'react-router-dom'

export const AuthFormLayout = () => {
  const location = useLocation()

  const isLogin = location.pathname === '/login'

  const title = isLogin ? 'Log in' : 'Sign up'

  return (
    <>
      <div className="font-medium text-xl -mb-0.5">{title}</div>
      <div className="text-muted-foreground text-[13px] mb-3 leading-[18px]">
        Automate your billing, create and test and any pricing strategy and uncover growth
        opportunities.
      </div>
      <Button variant="default" size="md" className="w-full" hasIcon>
        <img src="/company/google.svg" alt="Google" className="w-[19px] h-[19px] mb-0.5" />
        Continue with Google
      </Button>
      <Button variant="secondary" size="md" className="w-full" hasIcon>
        <img src="/company/github.svg" alt="Google" className="w-[19px] h-[19px] mb-0.5" />
        Continue with Github
      </Button>
      <Flex align="center" justify="center" className="gap-2 w-full mt-1">
        <div className="flex-grow">
          <Separator />
        </div>
        <div className="text-muted-foreground text-xs whitespace-nowrap">or</div>
        <div className="flex-grow">
          <Separator />
        </div>
      </Flex>
      <Outlet />
      <div className="text-[11px] text-center p-2 leading-4">
        <span className="text-muted-foreground mr-1">By proceeding, you agree to our </span>
        <Link to="/privacy" className="underline">
          Privacy Policy
        </Link>
        <span className="text-muted-foreground mx-1">and</span>
        <Link to="/terms" className="underline">
          Terms of service
        </Link>
      </div>
    </>
  )
}
