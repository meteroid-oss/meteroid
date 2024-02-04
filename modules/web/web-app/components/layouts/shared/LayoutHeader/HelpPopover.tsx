import { ButtonAlt as Button, PopoverAlt as Popover } from '@md/ui'
import {
  HelpCircle as IconHelpCircle,
  MessageCircle as IconMessageCircle,
} from 'lucide-react'
import { FC } from 'react'
import SVG from 'react-inlinesvg'

const HelpPopover: FC = () => {
  return (
    <Popover
      size="content"
      align="end"
      side="bottom"
      sideOffset={8}
      overlay={
        <div className="my-4 w-96 space-y-4">
          <div className="mb-4 space-y-2">
            <div className="mb-4 px-5">
              <h5 className="mb-2">Reach out to the team and community</h5>
            </div>
            <div className="px-5">
              <div
                className="relative space-y-2 overflow-hidden rounded px-5 py-4 pb-12 shadow-md"
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
                  <Button
                    type="secondary"
                    icon={<SVG src="/img/discord-icon.svg" className="h-4 w-4" />}
                  >
                    <span style={{ color: '#404EED' }}>Join us on Discord</span>
                  </Button>
                </a>
              </div>
            </div>
            <div className="px-5">
              <div className="relative space-y-2 overflow-hidden rounded px-5 py-4 pb-12 shadow-md">
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
                  <Button type="secondary" icon={<IconMessageCircle size={14} />}>
                    GitHub Discussions
                  </Button>
                </a>
              </div>
            </div>
          </div>
        </div>
      }
    >
      <Button
        as="span"
        type="default"
        icon={<IconHelpCircle size={16} strokeWidth={1.5} className="text-scale-1200" />}
      >
        Help / Feedback
      </Button>
    </Popover>
  )
}

export default HelpPopover
