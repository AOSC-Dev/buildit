<template>
  <v-app>
    <v-main class="bg-grey-lighten-3">
      <router-view />
    </v-main>
    <v-bottom-navigation>
      <v-btn v-for="link in links" :key="link.name" :to="link.url">
        <v-icon>{{ link.icon }}</v-icon>

        <span>{{ link.name }}</span>
      </v-btn>
    </v-bottom-navigation>
  </v-app>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  export default {
    data: () => ({
      links: [
        {
          'name': 'Dashboard',
          'url': '/',
          'icon': 'mdi:mdi-view-dashboard',
        },
        {
          'name': 'Pipelines',
          'url': '/pipelines',
          'icon': 'mdi:mdi-wrench',
        },
        {
          'name': 'Jobs',
          'url': '/jobs',
          'icon': 'mdi:mdi-rocket-launch',
        },
        {
          'name': 'Workers',
          'url': '/workers',
          'icon': 'mdi:mdi-laptop',
        },
      ],
    }),
  }
</script>