import { Flex } from '@ui/index'

import { AsciiArt } from './AsciiArt'

import type { SceneName } from './renderers'
import type { ReactNode } from 'react'

type EmptyStateSVGName = 'customers'

interface EmptyStateProps {
  title: string
  description: string
  /** A still illustration from /empty-state; `scene` takes its place when both are given. */
  imageName?: EmptyStateSVGName
  /** An animated ASCII scene in place of the illustration. */
  scene?: SceneName
  actions?: ReactNode
}

export const EmptyState = ({ title, description, imageName, scene, actions }: EmptyStateProps) => (
  <Flex direction="column" justify="center" align="center" className="h-full mx-auto max-w-[500px]">
    {scene ? (
      // At about half the art's own size (11px): a companion to the text, not the page's centrepiece.
      <AsciiArt scene={scene} fontSize={5.9} className="mb-5" />
    ) : imageName ? (
      <img
        src={`/empty-state/${imageName}.svg`}
        alt={imageName}
        className="mb-5"
        width={150}
        height={150}
      />
    ) : null}
    {/* The app's own label type, as PanelEmptyState titles its states. */}
    <div className="mb-1 text-center text-sm font-medium text-foreground">{title}</div>
    <p className="text-center text-muted-foreground text-sm font-light mb-5">{description}</p>
    {actions}
  </Flex>
)
