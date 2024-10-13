import { atom } from 'jotai'
import { focusAtom } from 'jotai-optics'
import { DeepPartial } from 'react-hook-form'

import { PriceComponent } from '@/lib/schemas/plans'

export const editedComponentAtom = atom<DeepPartial<PriceComponent>>({})
export const componentNameAtom = focusAtom(editedComponentAtom, optic => optic.prop('name'))
export const componentFeeAtom = focusAtom(editedComponentAtom, optic => optic.prop('fee'))
export const componentFeeTypeAtom = focusAtom(componentFeeAtom, optic =>
  optic.optional().prop('fee')
)
