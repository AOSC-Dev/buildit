<template>
  <router-link
    style="text-decoration: none; color: inherit;"
    :to="{ path: `/jobs/${jobId}` }">
    <v-icon v-if="status === 'success'" color="green" size="large">
      mdi:mdi-check-circle-outline
    </v-icon>
    <v-icon v-else-if="status === 'failed'" color="red" size="large">
      mdi:mdi-close-circle-outline
    </v-icon>
    <v-icon v-else-if="status === 'running'" color="blue" size="large">
      mdi:mdi-circle-slice-5
    </v-icon>
    <v-icon v-else-if="status === 'error'" color="red" size="large">
      mdi:mdi-alert-circle-outline
    </v-icon>
    <v-icon v-else color="grey" size="large">
      mdi:mdi-circle-slice-8
    </v-icon>
    <v-tooltip activator="parent" location="bottom">
      <template v-if="arch">
        Job #{{ jobId }} for {{ arch }}: {{ status }}
      </template>
      <template v-else>
        Job #{{ jobId }}: {{ status }}
      </template>
    </v-tooltip>
  </router-link>
</template>

<script lang="ts" setup>
  interface Props {
    jobId: number;
    status: string;
    arch: string;
  }

  defineProps<Props>();
</script>
