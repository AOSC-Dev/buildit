<template>
  <v-app>
    <v-app-bar flat>
      <v-container class="mx-auto d-flex align-center justify-center">
        <v-btn
          v-for="link in links"
          :key="link.name"
          :text="link.name"
          @click="$router.push(link.url)"
          variant="text"
        >
        </v-btn>
        <v-spacer></v-spacer>
        <v-btn text="Login" variant="text"></v-btn>
      </v-container>
    </v-app-bar>
    <v-main class="bg-grey-lighten-3">
      <router-view />
    </v-main>
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
          'url': '/'
        },
        {
          'name': 'Pipelines',
          'url': '/pipelines'
        },
        {
          'name': 'Jobs',
          'url': '/jobs'
        },
        {
          'name': 'Workers',
          'url': '/workers'
        },
      ],
    }),
  }
</script>