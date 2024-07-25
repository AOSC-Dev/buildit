<template>
  <v-container class="overflow-auto" style="height: 100%;font-family: monospace; white-space: nowarp;">
    <v-row v-if="loading">
      <v-spacer></v-spacer>
      <v-progress-circular indeterminate></v-progress-circular>
      <v-spacer></v-spacer>
    </v-row>
    <div v-html="html">
    </div>
  </v-container>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  import axios from 'axios';
  import { hostname } from '@/common';
  import { AnsiUp } from 'ansi_up';

  export default {
    mounted() {
      this.fetchData();
    },
    data: () => ({
      html: "",
      loading: true,
    }),
    methods: {
      async fetchData() {
        let name = (this.$route.params as { name: string }).name;
        let log = (await axios.get(`/logs/${name}`)).data;
        let ansi_up = new AnsiUp();
        let html = ansi_up.ansi_to_html(log);
        html = html.replaceAll("\r\n", " <br/> ");
        html = html.replaceAll("\n", " <br/> ");
        html = html.replaceAll("\r", " <br/> ");
        this.html = html;
        this.loading = false;
      }
    }
  }
</script>