// Appels API des notifications (ADR-0044) : canaux, règles, journal des envois, événements
// disponibles. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/settings/notifications'

export type Channel = Schemas['ChannelDto']
export type ChannelRequest = Schemas['ChannelRequest']
export type Rule = Schemas['RuleDto']
export type RuleRequest = Schemas['RuleRequest']
export type Delivery = Schemas['DeliveryDto']
export type ChannelTest = Schemas['TestDto']
export type Subscription = Schemas['SubscriptionDto']
export type Subscriptions = Schemas['SubscriptionsDto']
export type SubscriptionRequest = Schemas['SubscriptionRequest']

const SERVERS = '/admin/v1/servers'

export function useNotificationsApi() {
  return {
    events: () => http.get<string[]>(`${BASE}/events`).then((response) => response.data),

    channels: () => http.get<Channel[]>(`${BASE}/channels`).then((response) => response.data),

    createChannel: (body: ChannelRequest) => http.post<Channel>(`${BASE}/channels`, body).then((response) => response.data),

    updateChannel: (id: string, body: ChannelRequest) =>
      http.put<Channel>(`${BASE}/channels/${id}`, body).then((response) => response.data),

    removeChannel: (id: string) => http.delete<void>(`${BASE}/channels/${id}`).then(() => undefined),

    testChannel: (id: string) => http.post<ChannelTest>(`${BASE}/channels/${id}/test`).then((response) => response.data),

    rules: () => http.get<Rule[]>(`${BASE}/rules`).then((response) => response.data),

    createRule: (body: RuleRequest) => http.post<Rule>(`${BASE}/rules`, body).then((response) => response.data),

    updateRule: (id: string, body: RuleRequest) => http.put<Rule>(`${BASE}/rules/${id}`, body).then((response) => response.data),

    removeRule: (id: string) => http.delete<void>(`${BASE}/rules/${id}`).then(() => undefined),

    deliveries: (limit = 50) =>
      http.get<Delivery[]>(`${BASE}/deliveries`, { params: { limit } }).then((response) => response.data),

    /** Abonnements d'un serveur, avec les événements possibles et le droit de viser un canal partagé. */
    subscriptions: (server: string) =>
      http.get<Subscriptions>(`${SERVERS}/${server}/notifications`).then((response) => response.data),

    createSubscription: (server: string, body: SubscriptionRequest) =>
      http.post<Subscription>(`${SERVERS}/${server}/notifications`, body).then((response) => response.data),

    updateSubscription: (server: string, id: string, body: SubscriptionRequest) =>
      http.put<Subscription>(`${SERVERS}/${server}/notifications/${id}`, body).then((response) => response.data),

    removeSubscription: (server: string, id: string) =>
      http.delete<void>(`${SERVERS}/${server}/notifications/${id}`).then(() => undefined),

    testSubscription: (server: string, id: string) =>
      http.post<ChannelTest>(`${SERVERS}/${server}/notifications/${id}/test`).then((response) => response.data),

    serverDeliveries: (server: string, limit = 50) =>
      http.get<Delivery[]>(`${SERVERS}/${server}/notifications/deliveries`, { params: { limit } }).then((response) => response.data),
  }
}
