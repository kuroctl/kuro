<script setup lang="ts">
// Avatar du compte connecté : compte, thème, langue, déconnexion.
import { Languages, LogOut, Monitor, Moon, Sun, UserRound } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { useMe } from '@/features/account/account.queries'
import { useAuthApi } from '@/features/auth/auth.api'
import { useSessionStore } from '@/features/auth/session.store'
import { useApiError } from '@/shared/api/useApiError'
import { SUPPORTED_LOCALES, applyLocale, type Locale } from '@/shared/i18n'
import { useTheme } from '@/shared/theme/useTheme'
import { Avatar, AvatarFallback } from '@/shared/ui/avatar'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/shared/ui/dropdown-menu'

const { t, locale } = useI18n()
const router = useRouter()
const session = useSessionStore()
const { data: me } = useMe()
const { logout } = useAuthApi()
const { handleError } = useApiError()
const theme = useTheme()

const initials = computed(() =>
  (me.value?.user.display_name ?? '?')
    .split(/\s+/)
    .map((part) => part.charAt(0))
    .join('')
    .slice(0, 2)
    .toUpperCase(),
)

async function signOut() {
  try {
    await logout()
  } catch (error) {
    handleError(error)
  }
  session.close()
  await router.push('/login')
}
</script>

<template>
  <DropdownMenu>
    <DropdownMenuTrigger :aria-label="t('nav.account')" class="rounded-full outline-none">
      <Avatar class="size-10 transition-transform hover:scale-105">
        <AvatarFallback class="bg-primary/20 text-sm font-semibold text-primary">
          {{ initials }}
        </AvatarFallback>
      </Avatar>
    </DropdownMenuTrigger>
    <DropdownMenuContent side="right" align="end" class="w-60">
      <DropdownMenuLabel class="flex flex-col text-sm font-semibold tracking-normal text-popover-foreground normal-case">
        <span class="truncate">{{ me?.user.display_name }}</span>
        <span class="truncate text-xs font-normal text-muted-foreground">{{ me?.user.email }}</span>
      </DropdownMenuLabel>
      <DropdownMenuSeparator />
      <DropdownMenuItem @select="router.push('/account')">
        <UserRound />
        {{ t('nav.account') }}
      </DropdownMenuItem>
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>
          <Moon v-if="theme === 'dark'" />
          <Sun v-else-if="theme === 'light'" />
          <Monitor v-else />
          {{ t('preferences.theme') }}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent>
          <DropdownMenuRadioGroup v-model="theme">
            <DropdownMenuRadioItem value="dark">{{ t('preferences.dark') }}</DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="light">{{ t('preferences.light') }}</DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="auto">{{ t('preferences.system') }}</DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuSubContent>
      </DropdownMenuSub>
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>
          <Languages />
          {{ t('preferences.language') }}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent>
          <DropdownMenuRadioGroup
            :model-value="locale"
            @update:model-value="(value) => applyLocale(value as Locale)"
          >
            <DropdownMenuRadioItem v-for="code in SUPPORTED_LOCALES" :key="code" :value="code">
              {{ t(`preferences.languages.${code}`) }}
            </DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuSubContent>
      </DropdownMenuSub>
      <DropdownMenuSeparator />
      <DropdownMenuItem variant="destructive" @select="signOut">
        <LogOut />
        {{ t('auth.logout') }}
      </DropdownMenuItem>
    </DropdownMenuContent>
  </DropdownMenu>
</template>
