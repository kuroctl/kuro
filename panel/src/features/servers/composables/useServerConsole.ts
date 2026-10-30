// Console et stats d'un serveur en direct (ADR-0014).
//
// Un ticket à usage unique est demandé en REST puis envoyé comme premier message du WebSocket
// (jamais dans l'URL). La connexion est rouverte automatiquement : l'API la ferme au bout de la
// durée d'un jeton d'accès (4408), ou elle tombe avec le réseau. L'état reçu en direct met à jour
// le cache Vue Query du serveur.
import { useQueryClient } from '@tanstack/vue-query'
import { isAxiosError } from 'axios'
import { onScopeDispose, ref, shallowRef, toValue, watch, type MaybeRefOrGetter } from 'vue'

import { useApiError } from '@/shared/api/useApiError'

import { type Server, useServersApi } from '../servers.api'

export interface ConsoleLine {
  at_ms: number
  text: string
}

/** Mesure reçue, avec les débits réseau calculés depuis la précédente. */
export interface StatsSample {
  at: number
  cpu_millis: number
  memory_bytes: number
  memory_limit_bytes: number
  rx_per_sec: number
  tx_per_sec: number
}

type ConsoleMessage =
  | { type: 'lines'; lines: ConsoleLine[]; dropped?: number }
  | {
      type: 'stats'
      cpu_millis: number
      memory_bytes: number
      memory_limit_bytes: number
      network_rx_bytes: number
      network_tx_bytes: number
    }
  | { type: 'state'; state: Server['state']; reason?: string }

export type ConsoleStatus = 'connecting' | 'live' | 'reconnecting' | 'unavailable'

/** 5 minutes de mesures, une toutes les 2 s. */
export const STATS_WINDOW = 150
const CLOSE_NOT_FOUND = 4404
const CLOSE_EXPIRED = 4408
const MAX_RETRY_MS = 15_000

export function consoleUrl(id: string): string {
  const scheme = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${window.location.host}/api/admin/v1/servers/${id}/console`
}

export function useServerConsole(
  id: MaybeRefOrGetter<string>,
  onLines: (lines: ConsoleLine[], dropped: number) => void,
) {
  const { consoleTicket } = useServersApi()
  const { handleError } = useApiError()
  const client = useQueryClient()

  const status = ref<ConsoleStatus>('connecting')
  const samples = shallowRef<StatsSample[]>([])

  let socket: WebSocket | null = null
  let retry: ReturnType<typeof setTimeout> | undefined
  let attempt = 0
  // Incrémenté à chaque (re)démarrage : une connexion d'un serveur précédent est ignorée.
  let generation = 0
  // Compteurs réseau cumulés de la mesure précédente.
  let lastCounters: { rx: number; tx: number } | undefined

  function stop() {
    generation += 1
    clearTimeout(retry)
    socket?.close()
    socket = null
  }

  function schedule(delay: number) {
    const current = generation
    clearTimeout(retry)
    retry = setTimeout(() => {
      if (current === generation) void connect()
    }, delay)
  }

  function retryLater() {
    attempt += 1
    status.value = 'reconnecting'
    schedule(Math.min(1000 * 2 ** (attempt - 1), MAX_RETRY_MS))
  }

  async function connect() {
    const current = generation
    const server = toValue(id)
    let ticket: string
    try {
      ticket = (await consoleTicket(server)).ticket
    } catch (error) {
      if (current !== generation) return
      const code = isAxiosError(error) ? error.response?.status : undefined
      // Droits manquants ou serveur supprimé : inutile d'insister.
      if (code === 403 || code === 404) {
        handleError(error)
        status.value = 'unavailable'
        return
      }
      if (attempt === 0) handleError(error)
      retryLater()
      return
    }
    if (current !== generation) return

    const ws = new WebSocket(consoleUrl(server))
    socket = ws
    ws.onopen = () => ws.send(JSON.stringify({ type: 'auth', ticket }))
    ws.onmessage = (event: MessageEvent<string>) => {
      if (current !== generation) return
      status.value = 'live'
      attempt = 0
      receive(server, JSON.parse(event.data) as ConsoleMessage)
    }
    ws.onclose = (event) => {
      if (current !== generation) return
      socket = null
      if (event.code === CLOSE_NOT_FOUND) {
        status.value = 'unavailable'
      } else if (event.code === CLOSE_EXPIRED) {
        status.value = 'reconnecting'
        schedule(0)
      } else {
        retryLater()
      }
    }
  }

  function receive(server: string, message: ConsoleMessage) {
    switch (message.type) {
      case 'lines':
        onLines(message.lines, message.dropped ?? 0)
        break
      case 'stats': {
        const previous = samples.value.at(-1)
        const at = Date.now()
        const elapsed = previous ? (at - previous.at) / 1000 : 0
        const rate = (now: number, before: number | undefined) =>
          elapsed > 0 && before !== undefined ? Math.max(0, (now - before) / elapsed) : 0
        const sample: StatsSample = {
          at,
          cpu_millis: message.cpu_millis,
          memory_bytes: message.memory_bytes,
          memory_limit_bytes: message.memory_limit_bytes,
          rx_per_sec: rate(message.network_rx_bytes, lastCounters?.rx),
          tx_per_sec: rate(message.network_tx_bytes, lastCounters?.tx),
        }
        lastCounters = { rx: message.network_rx_bytes, tx: message.network_tx_bytes }
        samples.value = [...samples.value, sample].slice(-STATS_WINDOW)
        break
      }
      case 'state':
        client.setQueryData<Server>(['servers', server], (old) =>
          old ? { ...old, state: message.state, state_reason: message.reason ?? null } : old,
        )
        break
    }
  }

  watch(
    () => toValue(id),
    () => {
      stop()
      attempt = 0
      samples.value = []
      lastCounters = undefined
      status.value = 'connecting'
      void connect()
    },
    { immediate: true },
  )
  onScopeDispose(stop)

  return { status, samples }
}
