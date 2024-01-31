import { PlusIcon } from '@md/icons'

import { Placeholder, PlusContainer, StyledAvatarUpload } from './AvatarUpload.styled'

import type { FunctionComponent } from 'react'

interface AvatarUploadProps {
  initials?: string
}

const AvatarUpload: FunctionComponent<AvatarUploadProps> = ({ initials = 'JD' }) => {
  return (
    <StyledAvatarUpload>
      <Placeholder>
        {initials}
        <PlusContainer>
          <PlusIcon size={10} />
        </PlusContainer>
      </Placeholder>
    </StyledAvatarUpload>
  )
}

export default AvatarUpload
