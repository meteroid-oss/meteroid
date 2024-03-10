import { copyToClipboard } from '@/lib/helpers'
import { Button, Popover, PopoverTrigger, PopoverContent, Textarea } from '@ui2/components'
import { HelpCircle as IconHelpCircle, MessageCircle as IconMessageCircle } from 'lucide-react'
import { FC } from 'react'
import { toast } from 'sonner'

const copyEmail = () => {
  copyToClipboard('team@meteroid.com')
  toast.success('Email copied to clipboard')
}
const HelpPopover: FC = () => {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant={'ghost'} size="sm" className="h-9">
          <IconHelpCircle size={16} strokeWidth={1.5} className="mr-2" /> Help / Feedback
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-96">
        <div className="mb-4 space-y-2 px-5">
          <div className="mb-4 ">
            <h5 className="mb-1">Quick feedback</h5>
            <div className="flex flex-row gap-2 align-bottom">
              <Textarea className="" />
              <Button variant="secondary" size={'sm'} className="self-end">
                Send
              </Button>
            </div>
            <div className="text-xs">
              or{' '}
              <a
                className="underline"
                href="mailto:team@meteroid.com"
                target="_blank"
                rel="noopener noreferrer"
                onClick={() => copyEmail()}
              >
                email us
              </a>
            </div>
          </div>
          <div></div>
          <div className="mb-4 ">
            <h5 className="mb-2">Reach out to the team and community</h5>
          </div>
          <div>
            <div
              className="relative space-y-2 overflow-hidden rounded px-5 py-4 pb-12 shadow-md h-[100px]"
              style={{ background: '#404EED' }}
            >
              <a
                href="https://go.meteroid.com/discord"
                target="_blank"
                className="dark block cursor-pointer"
                rel="noreferrer"
              >
                <img
                  className="absolute left-0 top-0 opacity-50"
                  src="/img/support/discord-bg-small.jpg"
                  style={{ objectFit: 'cover' }}
                  alt="discord illustration header"
                />

                <Button hasIcon className="absolute left-3 top-3 opacity-80 bg-foreground">
                  <span style={{ color: '#404EED' }}>Join us on Discord !</span>
                </Button>
              </a>
            </div>
          </div>
          <div>
            <div className="relative space-y-2 overflow-hidden rounded px-5 py-4 pb-12 shadow-md h-[100px]">
              <a
                href="https://github.com/meteroid-oss/meteroid/discussions"
                target="_blank"
                className="block cursor-pointer"
                rel="noreferrer"
              >
                <img
                  className="absolute left-0 top-0 opacity-50"
                  src="/img/support/github-bg.jpg?v-1"
                  style={{
                    objectFit: 'cover',
                  }}
                  alt="discord illustration header"
                />
                <Button
                  variant="secondary"
                  hasIcon
                  className="absolute left-3 top-3 opacity-80 dark:bg-secondary bg-foreground dark:text-secondary-foreground text-secondary"
                >
                  <IconMessageCircle size={14} /> GitHub Discussions
                </Button>
              </a>
            </div>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  )
}

export default HelpPopover
