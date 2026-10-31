<script setup lang="ts">
// Paramètres › E-mail (ADR-0046) : le serveur SMTP du projet, pour les notifications par e-mail
// et les invitations des admins.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import MailSettingsForm from '@/features/mail/components/MailSettingsForm.vue'
import { useMailSettings } from '@/features/mail/mail.queries'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Skeleton } from '@/shared/ui/skeleton'

const { t } = useI18n()
const { can } = usePermissions()
const readable = computed(() => can('system.settings.read'))
const editable = computed(() => can('system.settings.update'))
const { data: settings, error } = useMailSettings(readable)
const { message } = useApiError()
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.settings.mail')" />
    <div class="grid max-w-4xl min-w-0 gap-6 p-6">
      <p v-if="!readable" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('settings.forbidden') }}
      </p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <Skeleton v-else-if="!settings" class="h-80" />
      <Card v-else>
        <CardHeader>
          <CardTitle class="flex items-center gap-2">
            {{ t('mail.title') }}
            <Badge v-if="settings.configured" variant="success">{{ t('mail.configured') }}</Badge>
            <Badge v-else variant="secondary">{{ t('mail.none') }}</Badge>
          </CardTitle>
          <CardDescription>{{ t('mail.help') }}</CardDescription>
        </CardHeader>
        <CardContent>
          <MailSettingsForm :settings="settings" :editable="editable" />
        </CardContent>
      </Card>
    </div>
  </div>
</template>
