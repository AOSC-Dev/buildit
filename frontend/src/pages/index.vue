<template>
  <v-container>
    <v-row>
      <v-col>
        <v-card height="200" title="Pipelines"></v-card>
      </v-col>
      <v-col>
        <v-card height="200" title="Jobs"></v-card>
      </v-col>
      <v-col>
        <v-card height="200" title="Workers"></v-card>
      </v-col>
    </v-row>
  </v-container>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  import axios from 'axios';
  export default {
    data: () => ({
      items: [
        'Workers',
        'Pipelines',
        'Jobs',
      ],
    }),
  }
</script>