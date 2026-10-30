<script setup lang="ts">
import { ShieldCheck, ShieldOff } from '@lucide/vue'
import { useI18n } from 'vue-i18n'

import PasswordForm from '@/features/account/components/PasswordForm.vue'
import { useMe } from '@/features/account/account.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'

const { t } = useI18n()
const { data: me } = useMe()
</script>

<template>
  <div>
    <!-- Racine unique : la transition entre pages ne sait animer qu'un élément. -->
    <PageHeader :title="t('subnav.account.security')" />
    <div v-if="me" class="max-w-xl p-6">
      <Card>
        <CardHeader>
          <CardTitle>{{ t('account.security.title') }}</CardTitle>
        </CardHeader>
        <CardContent class="flex flex-col gap-6 text-sm">
          <div class="flex items-center gap-2">
            <ShieldCheck v-if="me.user.totp_enabled" class="size-4 text-success" />
            <ShieldOff v-else class="size-4 text-warning" />
            <span>{{ t('account.security.totp') }}</span>
            <Badge :variant="me.user.totp_enabled ? 'secondary' : 'outline'" class="ml-auto">
              {{ me.user.totp_enabled ? t('account.security.totpEnabled') : t('account.security.totpDisabled') }}
            </Badge>
          </div>
          <div class="flex flex-col gap-3">
            <p class="font-medium">{{ t('account.security.password') }}</p>
            <PasswordForm />
          </div>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
