import { Button } from '@md/ui'
import { GithubIcon } from 'lucide-react'

export const StarGithub = () => {
  return (
    <a href="https://github.com/meteroid-oss/meteroid" target="_blank" rel="noreferrer">
      <Button size="icon" variant="ghost">
        <GithubIcon size={16} strokeWidth={1.5} />
      </Button>
    </a>
  )
}
