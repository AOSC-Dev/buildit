<template>
  <v-container style="height: 100%;font-family: monospace;">
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
    }),
    methods: {
      async fetchData() {
        let name = (this.$route.params as { name: string }).name;
        let log = (await axios.get(`/logs/${name}`)).data;
        let ansi_up = new AnsiUp();
        let html = ansi_up.ansi_to_html(log);
        html = html.replaceAll("\r\n", " <br/> ");
        html = html.replaceAll("\n", " <br/> ");
        this.html = html;
      }
    }
  }
</script>