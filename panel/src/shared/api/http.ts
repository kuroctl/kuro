// Instance axios unique : base `/api`, jeton d'accès, langue, et rafraîchissement de session.
//
// Sur un 401, la session est rafraîchie (un seul appel à la fois, garanti par le store `session`),
// puis chaque requête est rejouée une fois. La session est gérée par ce store, qui se branche ici
// via `configureAuth` (pas de dépendance circulaire).
//
// Sur un `403 elevation_required` (action protégée, ADR-0050), l'admin confirme son identité
// dans la fenêtre commune (branchée par `configureElevation`), puis la requête est rejouée une
// fois. S'il renonce, l'appel échoue avec `ElevationCancelled`, que `useApiError` passe sous
// silence.
import axios, { type AxiosError, type InternalAxiosRequestConfig } from 'axios'

export const http = axios.create({ baseURL: '/api', withCredentials: true })

interface AuthHooks {
  getAccessToken: () => string | null
  /** Nouveau jeton d'accès, ou `null` si la session est perdue. Partagé entre appels concurrents. */
  refresh: () => Promise<string | null>
  onSessionLost: () => void
}

let hooks: AuthHooks | null = null

/** Preuve demandée par l'API pour élever la session. */
export type ElevationMethod = 'totp' | 'password'

/** Ouvre la fenêtre de confirmation ; `true` une fois la session élevée, `false` si abandon. */
type ElevationHook = (method: ElevationMethod) => Promise<boolean>

let elevate: ElevationHook | null = null

/** L'admin a renoncé à confirmer son identité : l'action protégée n'a pas eu lieu. */
export class ElevationCancelled extends Error {
  constructor() {
    super('elevation cancelled')
    this.name = 'ElevationCancelled'
  }
}

/** Routes d'authentification : un 401 y est une réponse métier, jamais un jeton expiré. */
const AUTH_PATH = '/admin/v1/auth/'

export function configureAuth(authHooks: AuthHooks) {
  hooks = authHooks
}

export function configureElevation(hook: ElevationHook) {
  elevate = hook
}

export function setRequestLanguage(locale: string) {
  http.defaults.headers.common['Accept-Language'] = locale
}

http.interceptors.request.use((config) => {
  const token = hooks?.getAccessToken()
  if (token) config.headers.Authorization = `Bearer ${token}`
  return config
})

type RetriableConfig = InternalAxiosRequestConfig & { _retried?: boolean; _elevated?: boolean }

interface ElevationProblem {
  code?: string
  params?: { method?: string }
}

http.interceptors.response.use(undefined, async (error: AxiosError) => {
  const config = error.config as RetriableConfig | undefined
  const problem = error.response?.data as ElevationProblem | undefined
  if (error.response?.status === 403 && problem?.code === 'elevation_required' && config && !config._elevated && elevate) {
    if (!(await elevate(problem.params?.method === 'totp' ? 'totp' : 'password'))) {
      throw new ElevationCancelled()
    }
    config._elevated = true
    return http(config)
  }

  const expired =
    error.response?.status === 401 && config && !config._retried && !config.url?.startsWith(AUTH_PATH)
  if (!expired || !hooks) throw error

  // Jeton déjà renouvelé depuis l'envoi (réponse arrivée après un autre rafraîchissement) :
  // on rejoue avec le jeton courant, sans rafraîchir une seconde fois.
  const current = hooks.getAccessToken()
  const renewed = current !== null && config.headers.Authorization !== `Bearer ${current}`
  const token = renewed ? current : await hooks.refresh()
  if (!token) {
    hooks.onSessionLost()
    throw error
  }
  config._retried = true
  config.headers.Authorization = `Bearer ${token}`
  return http(config)
})
