import { fontFamilies } from './fonts.data'

import type { FunctionComponent } from 'react'

export const FontsPreload: FunctionComponent = () => {
  return (
    <>
      {fontFamilies
        .filter(font => font.preload)
        .map((font, fontIndex) => (
          <link
            key={fontIndex}
            rel="preload"
            href={`/fonts/${font.fontPath}.woff2`}
            as="font"
            type="font/woff2"
            crossOrigin="anonymous"
          />
        ))}
    </>
  )
}
