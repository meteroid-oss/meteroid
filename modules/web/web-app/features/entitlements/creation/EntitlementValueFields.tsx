import {
  Button,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Label,
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Textarea,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { InfoIcon, PlusIcon, Trash2Icon } from 'lucide-react'
import { Fragment } from 'react'
import { useFieldArray, useFormContext, useWatch } from 'react-hook-form'

import { ValuesTagInput } from '@/components/form'
import { AccessCheckInfo } from '@/features/credit/AccessCheckNote'

import { ResetPeriodFields } from './ResetPeriodFields'
import { limitSettingsApply, warningApplies } from './types'

import type { ResetPeriodType } from './types'

interface Props {
  featureType: 'boolean' | 'metered' | 'config'
  idPrefix: string // makes radio IDs unique when multiple instances exist on the same page
  // When true, a `select` config feature can edit its allowed options (only while creating a
  // new feature; existing features have a fixed option set).
  canEditConfigOptions?: boolean
  // When true, the config value type is chosen here (defining a new feature). When false, the
  // type is fixed by the feature and the selector is hidden — only the value editor shows.
  canPickConfigType?: boolean
}

function Info({ text }: { text: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <InfoIcon size={12} className="text-muted-foreground cursor-help" />
      </TooltipTrigger>
      <TooltipContent className="max-w-56">{text}</TooltipContent>
    </Tooltip>
  )
}

function Req() {
  return <span className="text-destructive ml-0.5">*</span>
}

function Opt() {
  return <span className="text-muted-foreground text-xs ml-1">(optional)</span>
}

export function EntitlementValueFields({
  featureType,
  idPrefix,
  canEditConfigOptions,
  canPickConfigType,
}: Props) {
  const { control } = useFormContext()
  const configKind = useWatch({ control, name: 'configKind' })
  const overageType = useWatch({ control, name: 'overageType' }) as string | undefined
  const limit = useWatch({ control, name: 'limit' }) as string | undefined
  const resetPeriodType = useWatch({ control, name: 'resetPeriodType' }) as
    | ResetPeriodType
    | undefined
  const hasLimit = limitSettingsApply(limit)
  const configOptions = useWatch({ control, name: 'configOptions' }) as string[] | undefined
  const { fields: entryFields, append: appendEntry, remove: removeEntry } = useFieldArray({
    control,
    name: 'configEntries',
  })
  const configEntries = useWatch({ control, name: 'configEntries' }) as
    | { key: string; type: string; value: string }[]
    | undefined

  if (featureType === 'config') {
    const kind = configKind ?? 'number'
    return (
      <Fragment key="config">
        {canPickConfigType && (
          <FormField
            control={control}
            name="configKind"
            render={({ field }) => (
              <FormItem>
                <FormLabel className="flex items-center gap-1">
                  Value type<Req />
                  <Info text="The shape of the configuration value. Map holds several typed key/value pairs (e.g. a limit per property); Raw JSON is the escape hatch for nested structures." />
                </FormLabel>
                <Select value={field.value ?? 'number'} onValueChange={field.onChange}>
                  <FormControl>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    <SelectItem value="number">Number</SelectItem>
                    <SelectItem value="boolean">Boolean</SelectItem>
                    <SelectItem value="text">Text</SelectItem>
                    <SelectItem value="map">Map (key/value)</SelectItem>
                    <SelectItem value="json">Raw JSON (advanced)</SelectItem>
                    <SelectItem value="select">Select (list)</SelectItem>
                  </SelectContent>
                </Select>
              </FormItem>
            )}
          />
        )}
        {kind === 'number' && (
          <FormField
            control={control}
            name="configNumber"
            render={({ field }) => (
              <FormItem>
                <FormLabel className="flex items-center gap-1">
                  Value<Req />
                  <Info text="A numeric configuration value, e.g. a hard limit like 3." />
                </FormLabel>
                <FormControl>
                  <Input type="number" placeholder="e.g. 3" {...field} value={field.value ?? ''} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        )}
        {kind === 'boolean' && (
          <FormField
            control={control}
            name="configBoolean"
            render={({ field }) => (
              <FormItem>
                <FormLabel className="flex items-center gap-1">Value<Req /></FormLabel>
                <FormControl>
                  <RadioGroup
                    value={field.value ? 'true' : 'false'}
                    onValueChange={v => field.onChange(v === 'true')}
                    className="flex gap-4"
                  >
                    <div className="flex items-center gap-1.5">
                      <RadioGroupItem value="true" id={`${idPrefix}-cfg-true`} />
                      <Label htmlFor={`${idPrefix}-cfg-true`} className="font-normal cursor-pointer">
                        True
                      </Label>
                    </div>
                    <div className="flex items-center gap-1.5">
                      <RadioGroupItem value="false" id={`${idPrefix}-cfg-false`} />
                      <Label htmlFor={`${idPrefix}-cfg-false`} className="font-normal cursor-pointer">
                        False
                      </Label>
                    </div>
                  </RadioGroup>
                </FormControl>
              </FormItem>
            )}
          />
        )}
        {kind === 'text' && (
          <FormField
            control={control}
            name="configText"
            render={({ field }) => (
              <FormItem>
                <FormLabel className="flex items-center gap-1">Value<Req /></FormLabel>
                <FormControl>
                  <Input placeholder="e.g. premium" {...field} value={field.value ?? ''} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        )}
        {kind === 'map' && (
          <FormField
            control={control}
            name="configEntries"
            render={() => (
              <FormItem>
                <FormLabel className="flex items-center gap-1">
                  Entries<Req />
                  <Info text="Typed key/value pairs stored as a JSON object — e.g. a limit per region or per property." />
                </FormLabel>
                <div className="flex flex-col gap-2">
                  {entryFields.map((f, i) => {
                    const rowType = configEntries?.[i]?.type ?? 'number'
                    return (
                      <div key={f.id} className="flex items-start gap-2">
                        <FormField
                          control={control}
                          name={`configEntries.${i}.key`}
                          render={({ field }) => (
                            <FormItem className="flex-1">
                              <FormControl>
                                <Input placeholder="key" {...field} value={field.value ?? ''} />
                              </FormControl>
                            </FormItem>
                          )}
                        />
                        <FormField
                          control={control}
                          name={`configEntries.${i}.type`}
                          render={({ field }) => (
                            <FormItem className="w-24 shrink-0">
                              <Select value={field.value ?? 'number'} onValueChange={field.onChange}>
                                <FormControl>
                                  <SelectTrigger>
                                    <SelectValue />
                                  </SelectTrigger>
                                </FormControl>
                                <SelectContent>
                                  <SelectItem value="number">Number</SelectItem>
                                  <SelectItem value="text">Text</SelectItem>
                                  <SelectItem value="boolean">Bool</SelectItem>
                                </SelectContent>
                              </Select>
                            </FormItem>
                          )}
                        />
                        <FormField
                          control={control}
                          name={`configEntries.${i}.value`}
                          render={({ field }) => (
                            <FormItem className="flex-1">
                              <FormControl>
                                {rowType === 'boolean' ? (
                                  <Select
                                    value={field.value === 'true' ? 'true' : 'false'}
                                    onValueChange={field.onChange}
                                  >
                                    <SelectTrigger>
                                      <SelectValue />
                                    </SelectTrigger>
                                    <SelectContent>
                                      <SelectItem value="true">True</SelectItem>
                                      <SelectItem value="false">False</SelectItem>
                                    </SelectContent>
                                  </Select>
                                ) : (
                                  <Input
                                    type={rowType === 'number' ? 'number' : 'text'}
                                    placeholder="value"
                                    {...field}
                                    value={field.value ?? ''}
                                  />
                                )}
                              </FormControl>
                            </FormItem>
                          )}
                        />
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="mt-0.5 shrink-0 text-muted-foreground hover:text-destructive"
                          onClick={() => removeEntry(i)}
                          aria-label="Remove entry"
                        >
                          <Trash2Icon size={14} />
                        </Button>
                      </div>
                    )
                  })}
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="self-start"
                    onClick={() => appendEntry({ key: '', type: 'number', value: '' })}
                  >
                    <PlusIcon size={14} className="mr-1" /> Add entry
                  </Button>
                </div>
                <FormMessage />
              </FormItem>
            )}
          />
        )}
        {kind === 'json' && (
          <FormField
            control={control}
            name="configJson"
            render={({ field }) => (
              <FormItem>
                <FormLabel className="flex items-center gap-1">
                  Value (JSON)<Req />
                  <Info text="A structured JSON value — an object holds several related settings in one entitlement." />
                </FormLabel>
                <FormControl>
                  <Textarea
                    rows={4}
                    placeholder='{ "maxAlerts": 3 }'
                    className="font-mono text-xs"
                    {...field}
                    value={field.value ?? ''}
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        )}
        {kind === 'select' && (
          <>
            {canEditConfigOptions && (
              <FormField
                control={control}
                name="configOptions"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel className="flex items-center gap-1">
                      Options<Req />
                      <Info text="Allowed values. Type a value and press Enter to add it; the entitlement value must be one of these." />
                    </FormLabel>
                    <FormControl>
                      <div className="min-h-9 rounded-md border border-input bg-transparent px-2 py-1.5">
                        <ValuesTagInput
                          value={field.value as string[] | undefined}
                          onChange={field.onChange}
                          placeholder="Type a value and press Enter"
                        />
                      </div>
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}
            <FormField
              control={control}
              name="configText"
              render={({ field }) => (
                <FormItem>
                  <FormLabel className="flex items-center gap-1">Value<Req /></FormLabel>
                  <Select value={field.value ?? ''} onValueChange={field.onChange}>
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue placeholder="Select a value" />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      {(configOptions ?? []).map(o => (
                        <SelectItem key={o} value={o}>
                          {o}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />
          </>
        )}
      </Fragment>
    )
  }

  if (featureType === 'boolean') {
    return (
      <Fragment key="boolean">
        <FormField
          control={control}
          name="boolEnabled"
          render={({ field }) => (
            <FormItem>
              <FormLabel className="flex items-center gap-1">
                Enabled<Req />
                <Info text="Whether access is granted (on) or explicitly revoked (off)." />
              </FormLabel>
              <FormControl>
                <RadioGroup
                  value={field.value ? 'true' : 'false'}
                  onValueChange={v => field.onChange(v === 'true')}
                  className="flex gap-4"
                >
                  <div className="flex items-center gap-1.5">
                    <RadioGroupItem value="true" id={`${idPrefix}-bool-on`} />
                    <Label htmlFor={`${idPrefix}-bool-on`} className="font-normal cursor-pointer">
                      Enabled
                    </Label>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <RadioGroupItem value="false" id={`${idPrefix}-bool-off`} />
                    <Label htmlFor={`${idPrefix}-bool-off`} className="font-normal cursor-pointer">
                      Disabled
                    </Label>
                  </div>
                </RadioGroup>
              </FormControl>
            </FormItem>
          )}
        />
      </Fragment>
    )
  }

  return (
    <Fragment key="metered">
      <FormField
        control={control}
        name="limit"
        render={({ field }) => (
          <FormItem>
            <FormLabel className="flex items-center gap-1">
              Limit<Opt />
              <Info text="Maximum usage allowed per reset period. Leave blank for unlimited." />
            </FormLabel>
            <FormControl>
              <Input type="number" min={0} placeholder="∞ Unlimited" {...field} />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
      <FormField
        control={control}
        name="groupBy"
        render={({ field }) => (
          <FormItem>
            <FormLabel className="flex items-center gap-1">
              Limit per property<Opt />
              <Info text="Apply the limit separately to each value of a metric property (e.g. project_id), instead of one limit on the total. Leave blank for a single limit." />
            </FormLabel>
            <FormControl>
              <Input placeholder="e.g. project_id" {...field} value={field.value ?? ''} />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
      <ResetPeriodFields />
      {hasLimit && (
        <FormField
          control={control}
          name="overageType"
          render={({ field }) => (
            <FormItem>
              <FormLabel className="flex items-center gap-1">
                At the limit
                <AccessCheckInfo />
              </FormLabel>
              <Select value={field.value ?? 'block'} onValueChange={field.onChange}>
                <FormControl>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                </FormControl>
                <SelectContent>
                  <SelectItem value="block">Hard limit: deny access</SelectItem>
                  <SelectItem value="allow">Soft limit: allow and bill overage</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {field.value === 'allow'
                  ? 'Access stays allowed past the limit; a price on the same metric bills the overage.'
                  : 'Access checks return denied once the limit, plus the grace if set, is reached.'}
              </p>
              <FormMessage />
            </FormItem>
          )}
        />
      )}
      {hasLimit && overageType !== 'allow' && (
        <FormField
          control={control}
          name="gracePct"
          render={({ field }) => (
            <FormItem>
              <FormLabel className="flex items-center gap-1">
                Grace<Opt />
                <Info text="Usage allowed beyond the limit before access checks return denied, in percent of the limit." />
              </FormLabel>
              <FormControl>
                <Input inputMode="numeric" placeholder="e.g. 10" {...field} value={field.value ?? ''} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      )}
      {warningApplies({ limit, resetPeriodType }) && (
        <FormField
          control={control}
          name="thresholds"
          render={({ field }) => (
            <FormItem>
              <FormLabel className="flex items-center gap-1">
                Alert thresholds<Opt />
                <Info text="Percentages of the limit, separated by commas: a webhook is sent once per window as usage reaches each. Reaching 100% always sends one." />
              </FormLabel>
              <FormControl>
                <Input placeholder="e.g. 50, 80, 95" {...field} value={field.value ?? ''} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      )}
      {hasLimit && resetPeriodType === 'slidingWindow' && (
        <p className="text-xs text-muted-foreground">
          A sliding window sends no threshold or limit webhooks; the entitlement check enforces it.
        </p>
      )}
    </Fragment>
  )
}
