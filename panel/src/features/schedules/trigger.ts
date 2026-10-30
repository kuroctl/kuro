// Déclencheur d'une tâche : les raccourcis de l'éditeur, traduits vers et depuis une expression
// cron à 5 champs. Une expression qu'aucun raccourci ne décrit reste « personnalisée ».

export type TriggerMode = 'hourly' | 'daily' | 'weekly' | 'custom'

export interface TriggerForm {
  mode: TriggerMode
  /** Toutes les `every` heures (`hourly`), à la minute `minute`. */
  every: number
  /** Heure et minute (`daily`, `weekly`). */
  hour: number
  minute: number
  /** Jours de la semaine, 0 = dimanche (`weekly`). */
  days: number[]
  /** Expression saisie (`custom`). */
  cron: string
}

export const WEEK = [1, 2, 3, 4, 5, 6, 0]

export function defaultTrigger(): TriggerForm {
  return { mode: 'daily', every: 6, hour: 4, minute: 0, days: [1, 3, 5], cron: '0 4 * * *' }
}

export function toCron(form: TriggerForm): string {
  switch (form.mode) {
    case 'hourly':
      return form.every === 1 ? `${form.minute} * * * *` : `${form.minute} */${form.every} * * *`
    case 'daily':
      return `${form.minute} ${form.hour} * * *`
    case 'weekly':
      return `${form.minute} ${form.hour} * * ${[...form.days].sort((a, b) => a - b).join(',') || '*'}`
    case 'custom':
      return form.cron.trim()
  }
}

const NUMBER = /^\d{1,2}$/

/** Retrouve le raccourci d'une expression ; sinon, mode personnalisé. */
export function fromCron(cron: string): TriggerForm {
  const form = { ...defaultTrigger(), mode: 'custom' as TriggerMode, cron }
  const [minute, hour, dom, month, dow, ...rest] = cron.trim().split(/\s+/)
  if (rest.length > 0 || !minute || !NUMBER.test(minute) || dom !== '*' || month !== '*') return form
  const m = Number(minute)
  if (hour === '*' && dow === '*') return { ...form, mode: 'hourly', every: 1, minute: m }
  const step = /^\*\/(\d{1,2})$/.exec(hour ?? '')
  if (step && dow === '*') return { ...form, mode: 'hourly', every: Number(step[1]), minute: m }
  if (!hour || !NUMBER.test(hour)) return form
  const h = Number(hour)
  if (dow === '*') return { ...form, mode: 'daily', hour: h, minute: m }
  if (dow && /^[0-6](,[0-6])*$/.test(dow)) return { ...form, mode: 'weekly', hour: h, minute: m, days: dow.split(',').map(Number) }
  return form
}

/** Fuseaux IANA proposés : ceux du navigateur, le sien en premier. */
export function timezones(): string[] {
  const own = Intl.DateTimeFormat().resolvedOptions().timeZone
  const all = typeof Intl.supportedValuesOf === 'function' ? Intl.supportedValuesOf('timeZone') : []
  return [own, ...all.filter((zone) => zone !== own && zone !== 'UTC'), 'UTC'].filter((zone, index, list) => list.indexOf(zone) === index)
}

/** Nom court d'un jour (0 = dimanche) dans la langue donnée. */
export function weekdayName(day: number, locale: string): string {
  // Le 25 octobre 2026 est un dimanche.
  return new Intl.DateTimeFormat(locale, { weekday: 'short' }).format(new Date(2026, 9, 25 + day))
}

const pad = (value: number) => String(value).padStart(2, '0')

/** Déclencheur en clair : « Tous les jours à 04:00 ». */
export function describeTrigger(cron: string, locale: string, t: (key: string, values?: Record<string, unknown>) => string): string {
  const form = fromCron(cron)
  const time = `${pad(form.hour)}:${pad(form.minute)}`
  switch (form.mode) {
    case 'hourly':
      return form.every === 1
        ? t('schedules.describe.hourly', { minute: pad(form.minute) })
        : t('schedules.describe.everyHours', { every: form.every, minute: pad(form.minute) })
    case 'daily':
      return t('schedules.describe.daily', { time })
    case 'weekly':
      return t('schedules.describe.weekly', {
        days: WEEK.filter((day) => form.days.includes(day))
          .map((day) => weekdayName(day, locale))
          .join(', '),
        time,
      })
    case 'custom':
      return t('schedules.describe.custom', { cron })
  }
}
