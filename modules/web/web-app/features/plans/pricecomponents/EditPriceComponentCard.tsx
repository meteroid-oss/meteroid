import { Button } from '@md/ui'
import { useAtom } from 'jotai'
import { CheckIcon, ChevronDownIcon, ChevronRightIcon, PencilIcon, XIcon } from 'lucide-react'
import { ReactNode, useState } from 'react'

import { feeTypeToHuman } from '@/features/plans/pricecomponents/utils'

import { componentFeeTypeAtom, componentNameAtom } from './atoms'

export interface EditPriceComponentCard {
  cancel: () => void
  submit: () => void
  children: ReactNode
}

export const EditPriceComponentCard = ({ cancel, submit, children }: EditPriceComponentCard) => {
  const [isCollapsed, setIsCollapsed] = useState(false)
  const [feeType] = useAtom(componentFeeTypeAtom)

  return (
    <div className="flex flex-col grow px-4 py-4 bg-popover border border-accent  shadow-md rounded-lg max-w-4xl">
      <div className="flex flex-row justify-between">
        <div className="mt-0.5 flex flex-row items-center ">
          <div
            className="mr-2 cursor-pointer select-none"
            onClick={() => setIsCollapsed(!isCollapsed)}
          >
            {isCollapsed ? (
              <ChevronRightIcon className="w-5 l-5 text-accent-foreground" />
            ) : (
              <ChevronDownIcon className="w-5 l-5 text-accent-foreground" />
            )}
          </div>
          <div className="flex items-center gap-2 ">
            <EditableComponentName />
            <span className="text-sm pl-4 text-muted-foreground">
              {feeType && <>({feeTypeToHuman(feeType)})</>}
            </span>
          </div>
        </div>
        <div className="flex flex-row items-center">
          <Button
            variant="ghost"
            className="font-bold py-1.5 !rounded-r-none bg-transparent "
            size="icon"
            onClick={cancel}
          >
            <XIcon size={16} strokeWidth={2} />
          </Button>
          <Button
            variant="ghost"
            className="font-bold py-1.5 !rounded-l-none text-success hover:text-success"
            onClick={submit}
            size="icon"
          >
            <CheckIcon size={16} strokeWidth={2} />
          </Button>
        </div>
      </div>
      <div className="flex flex-col grow px-7">
        <div className="mt-6 flex flex-col grow aria-hidden:hidden" aria-hidden={isCollapsed}>
          {children}
        </div>
      </div>
    </div>
  )
}

const EditableComponentName = () => {
  const [isEditing, setIsEditing] = useState(false)
  const [name, setName] = useAtom(componentNameAtom)

  return (
    <div className="flex flex-row items-center">
      {isEditing ? (
        <input
          className="bg-input py-1 px-1 text-base block w-full shadow-sm rounded-md ml-1 border-border"
          value={name}
          autoFocus
          onChange={e => setName(e.target.value)}
          onBlur={() => setIsEditing(false)}
          onKeyUp={e => e.key === 'Enter' && setIsEditing(false)}
        />
      ) : (
        <h4
          className="text-base text-accent-1 font-semibold flex space-x-2 items-center"
          onClick={() => setIsEditing(true)}
        >
          <span>{name}</span>
          <PencilIcon size={12} strokeWidth={2} />
        </h4>
      )}
    </div>
  )
}
