import { ButtonAlt } from '@md/ui'
import { GithubIcon } from 'lucide-react'

export const StarGithub = () => {
  return (
    <a
      href="https://github.com/meteroid-oss/meteroid"
      target="_blank"
      className="cursor-pointer flex items-center"
      rel="noreferrer"
    >
      <ButtonAlt
        type="default"
        icon={<GithubIcon size={16} strokeWidth={1.5} className="text-scale-1200" />}
      ></ButtonAlt>
    </a>
  )
}
