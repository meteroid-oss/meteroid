import { create } from '@bufbuild/protobuf'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  ComboboxFormField,
  DatePicker,
  Form,
  GenericFormField,
  InputFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectFormField,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  Switch,
  SwitchFormField,
  TextareaFormField,
  Toggle,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, PlusIcon } from 'lucide-react'
import { useEffect, useId, useMemo, useState } from 'react'
import { useFormContext, useWatch } from 'react-hook-form'
import { toast } from 'sonner'

import ConfirmationModal from '@/components/ConfirmationModal'
import { AdvancedSection, ChoiceCards, useHasFieldErrors } from '@/components/form'
import { PolicyFields } from '@/features/entitlements/PolicyFields'
import { FeatureCreateSheet } from '@/features/entitlements/features/FeatureCreateSheet'
import {
  EFFECT,
  type LevelKey,
  type Owner,
  type Source,
  allowedEffects,
  defaultEffect,
  effectOf,
  isAppCounted,
  isLimit,
  isOrdered,
  levelFields,
  levelOptions,
  levelSubject,
  levelTypeId,
  policyLabel,
  sourceOf,
} from '@/features/entitlements/model'
import { productSections } from '@/features/entitlements/productGroups'
import { OverridePreview } from '@/features/entitlements/rows/OverridePreview'
import { RowConsequence, hasConsequence } from '@/features/entitlements/rows/RowConsequence'
import { isOverrideSource } from '@/features/entitlements/rows/grantDisplay'
import { type GrantDraft, draftEffect, draftRow } from '@/features/entitlements/rows/grantDraft'
import {
  ENDS_OPTIONS,
  type RowForm,
  conditionFromForm,
  effectFromForm,
  rowSchema,
  rowToForm,
  specFromForm,
  updateFromForm,
  valueFromForm,
  valueIssue,
  withoutWindow,
} from '@/features/entitlements/rows/rowForm'
import { humanize, lower, useResourceTypes } from '@/features/resources/hooks'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  createEntitlement,
  listFeatures,
  updateEntitlement,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { FeatureOrder } from '@/rpc/api/entitlements/v1/entitlements_pb'
import {
  type Entitlement,
  EntitlementEffect,
  EntitlementSpecSchema,
  type Feature,
  FeatureKind,
  FeatureStatus,
  FeatureValueType,
} from '@/rpc/api/entitlements/v1/models_pb'

import type { ResourceType } from '@/rpc/api/resources/v1/models_pb'

export type RowScope = {
  owner: Owner
  /** A customer override on one stored resource. */
  resource?: { id: string; type: ResourceType; selector: Record<string, string> }
  /** The plan's "Sold for", which keeps across-all rows off plans sold for a type. */
  soldForTypeId?: string
  /** An add-on bought several times: Add is preselected. */
  multiInstance?: boolean
  /** The owner as the preview names it: "Each site on Growth". */
  ownerName?: string
}

/** A grant collected before its owner exists, returned through `onDraft`. */
export type DraftScope = {
  source: Source
  soldForTypeId?: string
  multiInstance?: boolean
}

type CommonProps = { feature?: Feature; onClose: () => void }
type SavedProps = RowScope &
  CommonProps & {
    existing?: Entitlement
    onDraft?: undefined
    /** A new override for a window: ends after a month, its reason asked for. */
    temporary?: boolean
  }
type DraftProps = DraftScope &
  CommonProps & {
    existing?: GrantDraft
    onDraft: (draft: GrantDraft) => void
    /** Features the picker leaves out: those already drafted. */
    excludeFeatureIds?: ReadonlySet<string>
  }
export type RowSheetProps = SavedProps | DraftProps

const isDraft = (p: RowSheetProps): p is DraftProps => p.onDraft !== undefined

/** The shared add/edit entitlement drawer, for every source; saves, or returns a draft. */
export const RowSheet = (props: RowSheetProps) => {
  const { soldForTypeId, multiInstance, feature: preset, onClose } = props
  const temporary = !isDraft(props) && !props.existing && !!props.temporary
  const queryClient = useQueryClient()
  const { types, byId: typesById } = useResourceTypes()
  const source: Source = isDraft(props) ? props.source : sourceOf(props.owner, props.resource?.id)
  const resource = isDraft(props) ? undefined : props.resource
  const existingDraft = isDraft(props) ? props.existing : undefined
  const existing = isDraft(props)
    ? props.existing &&
      draftRow(props.existing, draftEffect(props.existing, source, !!multiInstance))
    : props.existing
  // A draft's owner isn't in the customer's snapshot yet: no preview.
  const previewCustomerId =
    !isDraft(props) && isOverrideSource(source) && props.owner.case === 'customerId'
      ? props.owner.value
      : undefined
  const override = isOverrideSource(source)
  const [picked, setPicked] = useState<Feature | undefined>(preset ?? existingDraft?.feature)
  const [confirmingClose, setConfirmingClose] = useState(false)
  const [creatingFeature, setCreatingFeature] = useState(false)
  const feature = preset ?? picked

  // Promotional periods are the overrides'; a row that already has them keeps them visible.
  const showWindow = override || !!(existing?.validFrom || existing?.validUntil || existing?.reason)

  const defaults = (f: Feature | undefined) => ({
    level: (resource && resource.type.id !== f?.levelResourceTypeId
      ? `type:${resource.type.id}`
      : 'feature') as LevelKey,
    effect: defaultEffect(source, isOrdered(f), !!multiInstance),
  })
  const initial = (f: Feature | undefined, e?: Entitlement) => {
    const values = rowToForm(f, e, defaults(f))
    return temporary ? { ...values, ends: '1m' as const } : values
  }
  const form = useZodForm({ schema: rowSchema, defaultValues: initial(feature, existing) })
  const d = useWatch({ control: form.control }) as RowForm
  const isDirty = form.formState.isDirty
  const advancedErrors = useHasFieldErrors(form.control, ['validFrom', 'reason'])

  // A fresh pick is the new baseline: the form is clean until the user edits it.
  const pick = (f: Feature | undefined) => {
    setPicked(f)
    form.reset(initial(f))
  }

  const requestClose = () => (isDirty ? setConfirmingClose(true) : onClose())

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: listFeatures.parent, cardinality: undefined }),
    })
  const done = {
    onSuccess: () => invalidate().then(onClose),
    onError: (err: Error) => toast.error(err.message),
  }
  const createMutation = useMutation(createEntitlement, done)
  const updateMutation = useMutation(updateEntitlement, done)

  const onSubmit = form.handleSubmit(submitted => {
    if (!feature) return
    const data = showWindow ? submitted : withoutWindow(submitted)
    const issue = valueIssue(feature, data)
    if (issue) return form.setError('amount', { message: issue })
    if (isDraft(props)) {
      const spec = specFromForm(feature, data)
      // An untouched effect stays the source's default, applied by the server.
      const baseline = existing ? effectOf(existing) : defaults(feature).effect
      const effect =
        spec.effect === baseline
          ? (existingDraft?.spec.effect ?? EntitlementEffect.UNSPECIFIED)
          : spec.effect
      props.onDraft({ feature, spec: create(EntitlementSpecSchema, { ...spec, effect }) })
      return onClose()
    }
    if (props.existing) updateMutation.mutate(updateFromForm(feature, props.existing, data))
    else
      createMutation.mutate({
        entity: { EntityId: props.owner },
        spec: specFromForm(feature, data),
        resourceId: props.resource?.id,
      })
  })

  const effects = feature ? allowedEffects(source, isOrdered(feature)) : []
  const levels = feature ? levelOptions(feature, source, soldForTypeId, types) : []
  const conditionType = feature
    ? typesById.get(levelTypeId(d.level as LevelKey, feature) ?? '')
    : undefined
  const hasAttributes =
    !!conditionType && Object.values(conditionType.attributes).some(a => a.values.length > 0)
  const effect = effectFromForm(d)

  const title = resource
    ? `Override for ${lower(resource.type.name)} ${resource.selector[resource.type.keys[0]]}`
    : override
      ? existing
        ? 'Edit override'
        : temporary
          ? 'Add temporary override'
          : 'Add override'
      : source === 'default'
        ? existing
          ? 'Edit default'
          : 'Add default'
        : existing
          ? 'Edit entitlement'
          : 'Add entitlement'
  const submitLabel = existing
    ? 'Save'
    : override
      ? 'Add override'
      : source === 'default'
        ? 'Add default'
        : 'Add entitlement'

  // The row as it would save, for the preview; unset while its value is invalid.
  const formRow =
    feature && !valueIssue(feature, d)
      ? create(EntitlementSpecSchema, {
          value: valueFromForm(feature, d),
          effect,
          ...levelFields(d.level as LevelKey),
          condition: conditionFromForm(d),
        })
      : undefined

  return (
    <>
      <Sheet open onOpenChange={open => !open && requestClose()}>
        <SheetContent size="medium" className="p-0">
          <Form {...form}>
            <form onSubmit={onSubmit} className="flex h-full w-full flex-col">
              <SheetHeader className="ml-6 mt-4">
                <SheetTitle>{title}</SheetTitle>
              </SheetHeader>

              <div className="flex-1 overflow-y-auto">
                <div className="space-y-6 p-6">
                  {preset || existing ? (
                    <div className="flex flex-col gap-0.5">
                      <span className="text-sm font-medium">{feature?.name}</span>
                      {feature?.code && (
                        <span className="font-mono text-xs text-muted-foreground">
                          {feature.code}
                        </span>
                      )}
                    </div>
                  ) : (
                    <FeaturePicker
                      onPick={pick}
                      onCreate={() => setCreatingFeature(true)}
                      exclude={isDraft(props) ? props.excludeFeatureIds : undefined}
                    />
                  )}

                  {feature && (
                    <>
                      {effects.length > 1 && (
                        <GenericFormField
                          control={form.control}
                          name="effect"
                          label="Effect"
                          render={({ field }) => (
                            <ChoiceCards
                              value={field.value}
                              onChange={field.onChange}
                              columns={override ? 1 : 2}
                              options={effects.map(e => ({
                                value: EntitlementEffect[e],
                                label: EFFECT[e].label,
                                hint: EFFECT[e].hint,
                              }))}
                            />
                          )}
                        />
                      )}

                      <ValueField feature={feature} />

                      {levels.length > 1 && !existing && (
                        <GenericFormField
                          control={form.control}
                          name="level"
                          label="Applies to"
                          render={({ field }) => (
                            <Select
                              value={field.value}
                              onValueChange={v => {
                                field.onChange(v)
                                form.setValue('applies', 'always', { shouldDirty: true })
                                form.setValue('clauses', {}, { shouldDirty: true })
                              }}
                            >
                              <SelectTrigger ref={field.ref}>
                                <SelectValue />
                              </SelectTrigger>
                              <SelectContent>
                                {levels.map(k => (
                                  <SelectItem key={k} value={k}>
                                    {levelSubject(k, feature, typesById)}
                                  </SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                          )}
                        />
                      )}

                      {conditionType && hasAttributes && source !== 'resource' && (
                        <AppliesField type={conditionType} />
                      )}

                      {isLimit(feature) && source !== 'default' && (
                        <PolicySection
                          feature={feature}
                          canOverride={effect === EntitlementEffect.SET}
                        />
                      )}

                      {showWindow && (
                        <>
                          <div className="grid grid-cols-2 gap-3">
                            <SelectFormField control={form.control} name="ends" label="Ends">
                              {ENDS_OPTIONS.map(([value, label]) => (
                                <SelectItem key={value} value={value}>
                                  {label}
                                </SelectItem>
                              ))}
                            </SelectFormField>
                            {d.ends === 'date' && (
                              <GenericFormField
                                control={form.control}
                                name="validUntil"
                                label="End date"
                                render={({ field }) => (
                                  <DatePicker
                                    mode="single"
                                    captionLayout="dropdown"
                                    className="w-full"
                                    placeholder="Pick a date"
                                    date={field.value}
                                    onSelect={field.onChange}
                                    clearable
                                  />
                                )}
                              />
                            )}
                          </div>

                          <AdvancedSection
                            label="More"
                            defaultOpen={temporary || !!(existing?.validFrom || existing?.reason)}
                            forceOpen={advancedErrors}
                          >
                            <GenericFormField
                              control={form.control}
                              name="validFrom"
                              label="Starts"
                              render={({ field }) => (
                                <DatePicker
                                  mode="single"
                                  captionLayout="dropdown"
                                  className="w-full"
                                  placeholder="Now"
                                  date={field.value}
                                  onSelect={field.onChange}
                                  clearable
                                />
                              )}
                            />
                            <InputFormField
                              control={form.control}
                              name="reason"
                              label="Reason (optional)"
                              placeholder={
                                temporary ? 'e.g. Pilot extension' : 'e.g. Contract amendment'
                              }
                            />
                          </AdvancedSection>
                        </>
                      )}

                      {!isDraft(props) && hasConsequence(source) && (
                        <RowConsequence
                          feature={feature}
                          source={source}
                          owner={props.owner}
                          ownerName={props.ownerName}
                          existingId={props.existing?.id}
                          row={formRow}
                          multiInstance={!!multiInstance}
                        />
                      )}

                      {previewCustomerId && (
                        <OverridePreview
                          customerId={previewCustomerId}
                          resource={resource}
                          feature={feature}
                          typeId={levelTypeId(d.level as LevelKey, feature)}
                          existingId={isDraft(props) ? undefined : props.existing?.id}
                          row={formRow}
                        />
                      )}

                      <Warnings feature={feature} effect={effect} level={d.level as LevelKey} />
                    </>
                  )}
                </div>
              </div>

              <SheetFooter className="border-t border-border p-3">
                <Button type="button" variant="outline" onClick={requestClose}>
                  Cancel
                </Button>
                <Button
                  type="submit"
                  disabled={!feature || createMutation.isPending || updateMutation.isPending}
                >
                  {submitLabel}
                </Button>
              </SheetFooter>
            </form>
          </Form>
        </SheetContent>
      </Sheet>

      <ConfirmationModal
        visible={confirmingClose}
        danger
        header="Discard unsaved changes?"
        buttonLabel="Discard"
        onSelectCancel={() => setConfirmingClose(false)}
        onSelectConfirm={() => {
          setConfirmingClose(false)
          onClose()
        }}
      />

      {creatingFeature && (
        <FeatureCreateSheet
          onClose={(created?: Feature) => {
            setCreatingFeature(false)
            if (created) pick(created)
          }}
        />
      )}
    </>
  )
}

export const FeaturePicker = ({
  onPick,
  onCreate,
  exclude,
}: {
  onPick: (f: Feature | undefined) => void
  onCreate: () => void
  exclude?: ReadonlySet<string>
}) => {
  const { control } = useFormContext<RowForm>()
  const [search, setSearch] = useState('')
  const debounced = useDebounceValue(search, 300)
  const query = useQuery(listFeatures, {
    pagination: { page: 0, perPage: 100 },
    statuses: [FeatureStatus.ACTIVE, FeatureStatus.DISABLED],
    search: debounced || undefined,
    order: FeatureOrder.PRODUCT,
  })
  const features = useMemo(
    () => (query.data?.features ?? []).filter(f => !exclude?.has(f.id)),
    [query.data, exclude]
  )
  const groups: { name?: string; items: Feature[] }[] = productSections(features, f => f.product) ?? [
    { items: features },
  ]
  const featureId = useWatch({ control, name: 'featureId' })
  useEffect(() => {
    const f = features.find(x => x.id === featureId)
    if (f) onPick(f)
  }, [featureId]) // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <ComboboxFormField
      control={control}
      name="featureId"
      label="Feature"
      placeholder="Pick a feature"
      hasSearch
      shouldFilter={false}
      onSearchChange={setSearch}
      unit="feature"
      action={
        <button
          type="button"
          className="flex w-full items-center gap-2 px-2 py-1.5 text-sm hover:bg-accent"
          onClick={onCreate}
        >
          <PlusIcon size={14} /> New feature
        </button>
      }
      options={groups.flatMap(group =>
        group.items.map(f => ({
          value: f.id,
          label: (
            <span className="flex min-w-0 items-baseline gap-2">
              <span className="truncate">{f.name}</span>
              <span className="truncate font-mono text-xs text-muted-foreground">{f.code}</span>
            </span>
          ),
          keywords: [f.name, f.code],
          group: group.name,
        }))
      )}
    />
  )
}

const LIMIT_MODES = [
  ['amount', 'Amount'],
  ['unlimited', 'Unlimited'],
  ['off', 'Off'],
] as const

export const ValueField = ({ feature }: { feature: Feature }) => {
  const { control } = useFormContext<RowForm>()
  const mode = useWatch({ control, name: 'mode' })
  const id = useId()
  const unit = feature.unit || undefined

  if (feature.kind === FeatureKind.BOOLEAN)
    return (
      <GenericFormField
        control={control}
        name="mode"
        label="Value"
        render={({ field }) => (
          <div className="flex h-9 items-center gap-2">
            <Switch
              id={`${id}-on`}
              checked={field.value !== 'off'}
              onCheckedChange={on => field.onChange(on ? 'on' : 'off')}
            />
            <Label htmlFor={`${id}-on`} className="cursor-pointer font-normal">
              {field.value === 'off' ? 'Off' : 'On'}
            </Label>
          </div>
        )}
      />
    )

  if (feature.kind === FeatureKind.LIMIT)
    return (
      <div className="space-y-2">
        <GenericFormField
          control={control}
          name="mode"
          label="Value"
          render={({ field }) => (
            <RadioGroup
              value={field.value}
              onValueChange={field.onChange}
              className="flex h-8 items-center gap-4"
            >
              {LIMIT_MODES.map(([value, label]) => (
                <div key={value} className="flex items-center space-x-2">
                  <RadioGroupItem value={value} id={`${id}-${value}`} />
                  <Label htmlFor={`${id}-${value}`} className="cursor-pointer font-normal">
                    {label}
                  </Label>
                </div>
              ))}
            </RadioGroup>
          )}
        />
        {mode === 'amount' && (
          <InputFormField
            control={control}
            name="amount"
            inputMode="decimal"
            placeholder="Amount"
            rightText={unit}
            aria-label="Amount"
          />
        )}
      </div>
    )

  switch (feature.value?.type) {
    case FeatureValueType.NUMBER:
      return (
        <InputFormField
          control={control}
          name="amount"
          label="Value"
          inputMode="decimal"
          rightText={unit}
        />
      )
    case FeatureValueType.SELECT:
      return (
        <SelectFormField control={control} name="amount" label="Value" placeholder="Choose a value">
          {(feature.value?.options ?? []).map(o => (
            <SelectItem key={o} value={o}>
              {o}
            </SelectItem>
          ))}
        </SelectFormField>
      )
    case FeatureValueType.JSON:
      return (
        <TextareaFormField
          control={control}
          name="amount"
          label="Value"
          rows={4}
          className="font-mono text-xs"
        />
      )
    default:
      return <InputFormField control={control} name="amount" label="Value" rightText={unit} />
  }
}

const AppliesField = ({ type }: { type: ResourceType }) => {
  const { control, setValue, getFieldState, trigger } = useFormContext<RowForm>()
  const [applies, clauses] = useWatch({ control, name: ['applies', 'clauses'] })
  const id = useId()

  const toggle = (key: string, value: string, on: boolean) => {
    const selected = clauses[key] ?? []
    const next = on ? [...selected, value] : selected.filter(v => v !== value)
    setValue('clauses', { ...clauses, [key]: next }, { shouldDirty: true })
    if (getFieldState('applies').error) void trigger('applies')
  }

  return (
    <GenericFormField
      control={control}
      name="applies"
      label="Applies"
      render={({ field }) => (
        <div className="space-y-3">
          <RadioGroup
            value={field.value}
            onValueChange={v => {
              field.onChange(v)
              if (v === 'always') setValue('clauses', {}, { shouldDirty: true })
            }}
            className="flex h-8 items-center gap-4"
          >
            <div className="flex items-center space-x-2">
              <RadioGroupItem value="always" id={`${id}-always`} />
              <Label htmlFor={`${id}-always`} className="cursor-pointer font-normal">
                Always
              </Label>
            </div>
            <div className="flex items-center space-x-2">
              <RadioGroupItem value="when" id={`${id}-when`} />
              <Label htmlFor={`${id}-when`} className="cursor-pointer font-normal">
                Only when…
              </Label>
            </div>
          </RadioGroup>
          {applies === 'when' && (
            <div className="space-y-3 rounded-md border border-border bg-muted/40 p-3">
              {Object.entries(type.attributes)
                .filter(([, { values }]) => values.length > 0)
                .map(([attribute, { values }]) => {
                  const key = `${type.code}.${attribute}`
                  const selected = clauses[key] ?? []
                  return (
                    <div key={key} className="flex flex-wrap items-center gap-1.5">
                      <span className="text-xs font-medium">{humanize(attribute)}</span>
                      <span className="mr-1 text-xs text-muted-foreground">is one of</span>
                      {[...new Set([...values, ...selected])].map(v => (
                        <Toggle
                          key={v}
                          size="sm"
                          variant="outline"
                          pressed={selected.includes(v)}
                          onPressedChange={on => toggle(key, v, on)}
                          className="h-7 rounded-full px-2.5 text-xs font-normal data-[state=on]:border-primary data-[state=on]:bg-primary/10 data-[state=on]:text-primary"
                        >
                          {v}
                        </Toggle>
                      ))}
                    </div>
                  )
                })}
            </div>
          )}
        </div>
      )}
    />
  )
}

/** The policy is the feature's; only a Set to row may override it. */
const PolicySection = ({ feature, canOverride }: { feature: Feature; canOverride: boolean }) => {
  const { control } = useFormContext<RowForm>()
  const overridePolicy = useWatch({ control, name: 'overridePolicy' })
  const appCounted = isAppCounted(feature)
  const overriding = canOverride && overridePolicy
  const label = policyLabel(feature.policy, appCounted)
  return (
    <div className="space-y-2">
      <Label className="text-xs font-normal dark:text-muted-foreground">Policy</Label>
      {!overriding && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm">{label.charAt(0).toUpperCase() + label.slice(1)}</span>
          <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
            From the feature
          </Badge>
        </div>
      )}
      {canOverride && (
        <SwitchFormField
          control={control}
          name="overridePolicy"
          label="Override policy"
          containerClassName="py-1"
        />
      )}
      {overriding && <PolicyFields name="policy" appCounted={appCounted} />}
    </div>
  )
}

function Warnings({
  feature,
  effect,
  level,
}: {
  feature: Feature
  effect: EntitlementEffect
  level: LevelKey
}) {
  const { byId } = useResourceTypes()
  const own = feature.levelResourceTypeId ? byId.get(feature.levelResourceTypeId) : undefined
  const notes: string[] = []
  if (
    own &&
    level === 'customer' &&
    (effect === EntitlementEffect.RAISE_TO || effect === EntitlementEffect.ADD)
  )
    notes.push(
      `Needs a ${isLimit(feature) ? 'limit' : 'value'} across all ${lower(own.pluralName)} on the plan.`
    )
  if (!notes.length) return null
  return (
    <Alert variant="warning">
      <AlertTriangle className="h-4 w-4" />
      <AlertDescription className="flex flex-col gap-1">
        {notes.map(n => (
          <span key={n}>{n}</span>
        ))}
      </AlertDescription>
    </Alert>
  )
}
