import { Flex } from '@ui/components'
import { useEffect, useState } from 'react'
import { Link, Outlet, useLocation } from 'react-router-dom'

import { MeteroidTitle } from '@/components/svg'
import { useForceTheme } from 'providers/ThemeProvider'

type StarStyle = {
  left: string
  top: string
  width: string
  height: string
  backgroundColor: string
  animationDuration: string
  animationDelay: string
}

const Star = ({ style }: { style: React.CSSProperties }) => (
  <div className="absolute rounded-full" style={style} />
)

export const AuthLayout = () => {
  useForceTheme('dark')
  const location = useLocation()
  const isLogin = location.pathname === '/login'
  const [stars, setStars] = useState<StarStyle[]>([])

  useEffect(() => {
    // Create an array of random stars
    const generateStars = () => {
      const newStars = []
      const count = Math.floor(Math.random() * 50) + 150 // Between 150-200 stars

      for (let i = 0; i < count; i++) {
        // Random position
        const left = Math.random() * 100
        const top = Math.random() * 100

        // Skip stars that would appear in the center Outlet area
        // This is an approximate calculation to avoid the center area
        const isCenterX = left > 30 && left < 70
        const isCenterY = top > 20 && top < 80
        if (isCenterX && isCenterY) continue

        // Randomize between white and gray with varying opacity
        const color =
          Math.random() > 0.5
            ? `rgba(255, 255, 255, ${Math.random() * 0.5 + 0.2})` // white with 0.2-0.7 opacity
            : `rgba(180, 180, 180, ${Math.random() * 0.4 + 0.1})` // gray with 0.1-0.5 opacity

        newStars.push({
          left: `${left}%`,
          top: `${top}%`,
          width: `${Math.random() * 2 + 0.5}px`,
          height: `${Math.random() * 2 + 0.5}px`,
          backgroundColor: color,
          animationDuration: `${Math.random() * 3 + 2}s`,
          animationDelay: `${Math.random() * 2}s`,
        })
      }
      setStars(newStars)
    }

    generateStars()
  }, [])

  return (
    <div
      className="dark min-h-screen flex flex-col overflow-hidden relative"
      style={{
        background:
          'linear-gradient(0deg, #000 0%, #000 100%), linear-gradient(0deg, #0C0C0C 0%, #0C0C0C 100%), #111',
      }}
    >
      {stars.map((style, index) => (
        <Star key={index} style={style} />
      ))}
      <div className="p-6">
        <Flex justify="between" align="center">
          <MeteroidTitle forceTheme="dark" />
          <div className="text-xs">
            <span className="text-muted-foreground mr-1">
              {isLogin ? `Don't have an account?` : 'Already have an account?'}
            </span>

            <Link to={isLogin ? 'registration' : 'login'}>
              <span className="underline">{isLogin ? 'Sign up' : 'Log in'}</span>
            </Link>
          </div>
        </Flex>
      </div>

      <Flex justify="center" align="center" className="flex-grow pb-20">
        <Flex direction="column" className="p-10 w-96 gap-3 text-start relative z-10">
          <Outlet />
        </Flex>
      </Flex>
      <div className="absolute bottom-0 w-full">
        <img
          src="/sliced.svg"
          alt="Decorative slice"
          className="w-full"
          style={{
            pointerEvents: 'none',
          }}
        />
      </div>
    </div>
  )
}
