import { zodResolver } from '@hookform/resolvers/zod'
import { createFormControl } from 'react-hook-form'
import { describe, expect, it } from 'vitest'

import {
  portalCreditSchema,
  portalCreditUpdate,
  portalCreditValues,
} from './portalCreditForm'

import type { PortalCreditValues } from './portalCreditForm'

const entity = (overrides: Partial<Parameters<typeof portalCreditValues>[0]> = {}) => ({
  id: 'ive_1',
  portalTopUpEnabled: false,
  portalAutoTopUpEnabled: false,
  portalBudgetsEnabled: false,
  ...overrides,
})

// Drives the form as PortalCreditSettings does: reset only when the entity id or a stored value
// changes (its effect dependencies).
// `_getDirty` is what useForm derives `formState.isDirty` (the unsaved-changes bar) from.
const settingsForm = () => {
  const form = createFormControl<PortalCreditValues>({
    resolver: zodResolver(portalCreditSchema),
  })
  let snapshot: string | undefined
  const render = (loaded: ReturnType<typeof entity> | undefined) => {
    const next = JSON.stringify([loaded?.id, portalCreditValues(loaded)])
    if (next !== snapshot) {
      snapshot = next
      form.reset(portalCreditValues(loaded))
    }
  }
  const submit = () =>
    new Promise<ReturnType<typeof portalCreditUpdate>>((resolve, reject) =>
      form
        .handleSubmit(
          values => resolve(portalCreditUpdate(values)),
          errors => reject(errors)
        )()
        .catch(reject)
    )
  return { form, render, submit }
}

describe('portal credit settings form', () => {
  it('keeps a toggle across re-renders and refetches of the same entity, and saves it', async () => {
    const { form, render, submit } = settingsForm()
    render(entity())
    expect(form.getValues('portalTopUpEnabled')).toBe(false)

    form.setValue('portalTopUpEnabled', true, { shouldDirty: true })
    render(entity())
    render(entity())

    expect(form.getValues('portalTopUpEnabled')).toBe(true)
    expect(form.control._getDirty()).toBe(true)
    await expect(submit()).resolves.toEqual({
      portalTopUpEnabled: true,
      portalAutoTopUpEnabled: false,
      portalBudgetsEnabled: false,
    })
  })

  it('turns a stored switch off', async () => {
    const { form, render, submit } = settingsForm()
    render(entity({ portalBudgetsEnabled: true }))
    form.setValue('portalBudgetsEnabled', false, { shouldDirty: true })
    render(entity({ portalBudgetsEnabled: true }))

    expect(form.control._getDirty()).toBe(true)
    await expect(submit()).resolves.toMatchObject({ portalBudgetsEnabled: false })
  })

  it('takes the saved values once the entity reloads with them', () => {
    const { form, render } = settingsForm()
    render(entity())
    form.setValue('portalAutoTopUpEnabled', true, { shouldDirty: true })
    render(entity({ portalAutoTopUpEnabled: true }))

    expect(form.getValues('portalAutoTopUpEnabled')).toBe(true)
    expect(form.control._getDirty()).toBe(false)
  })

  it('drops unsaved edits when another invoicing entity is selected', () => {
    const { form, render } = settingsForm()
    render(entity())
    form.setValue('portalTopUpEnabled', true, { shouldDirty: true })
    render(entity({ id: 'ive_2' }))

    expect(form.getValues('portalTopUpEnabled')).toBe(false)
    expect(form.control._getDirty()).toBe(false)
  })
})

describe('saving', () => {
  it('takes the saved entity at once, so the bar does not offer Save again', () => {
    const { form, render } = settingsForm()
    render(entity())
    form.setValue('portalTopUpEnabled', true, { shouldDirty: true })
    form.reset(portalCreditValues(entity({ portalTopUpEnabled: true })))

    expect(form.control._getDirty()).toBe(false)
    expect(form.getValues('portalTopUpEnabled')).toBe(true)
  })
})
