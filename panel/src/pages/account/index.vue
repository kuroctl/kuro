<script setup lang="ts">
import { useI18n } from 'vue-i18n'

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
    <PageHeader :title="t('subnav.account.profile')" />
    <div v-if="me" class="max-w-xl p-6">
      <Card>
        <CardHeader>
          <CardTitle>{{ t('account.profile.title') }}</CardTitle>
        </CardHeader>
        <CardContent class="flex flex-col gap-4 text-sm">
          <div>
            <p class="text-muted-foreground">{{ t('account.profile.name') }}</p>
            <p class="font-medium">{{ me.user.display_name }}</p>
          </div>
          <div>
            <p class="text-muted-foreground">{{ t('account.profile.email') }}</p>
            <p class="font-medium">{{ me.user.email }}</p>
          </div>
          <div>
            <p class="mb-1 text-muted-foreground">{{ t('account.profile.roles') }}</p>
            <div class="flex flex-wrap gap-1.5">
              <Badge v-for="role in me.user.roles" :key="role.id" variant="secondary">{{ role.name }}</Badge>
              <span v-if="me.user.roles.length === 0" class="text-muted-foreground">{{ t('account.profile.noRole') }}</span>
            </div>
          </div>
          <p class="text-muted-foreground">
            {{
              me.all_permissions
                ? t('account.profile.superAdmin')
                : t('account.profile.permissionCount', me.permissions.length)
            }}
          </p>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
