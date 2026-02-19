import { atom } from 'jotai'

import type { ComponentFeeType } from '@/features/pricing/conversions'

export const componentNameAtom = atom('')
export const componentFeeTypeAtom = atom<ComponentFeeType>('rate')
