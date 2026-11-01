<script setup lang="ts">
import { useI18n } from 'vue-i18n'

import PasswordForm from '@/features/account/components/PasswordForm.vue'
import SshKeysCard from '@/features/account/components/SshKeysCard.vue'
import TotpCard from '@/features/account/components/TotpCard.vue'
import { useMe } from '@/features/account/account.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'

const { t } = useI18n()
const { data: me } = useMe()
</script>

<template>
  <div>
    <!-- Racine unique : la transition entre pages ne sait animer qu'un élément. -->
    <PageHeader :title="t('subnav.account.security')" />
    <div v-if="me" class="grid max-w-xl gap-6 p-6">
      <TotpCard :profile="me" />
      <Card>
        <CardHeader>
          <CardTitle>{{ t('account.security.password') }}</CardTitle>
        </CardHeader>
        <CardContent class="text-sm">
          <PasswordForm />
        </CardContent>
      </Card>
      <SshKeysCard />
    </div>
  </div>
</template>
