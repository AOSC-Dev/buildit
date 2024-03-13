<template>
  <v-container style="height: 100%;">
    <div ref="element" style="height: 100%;">
    </div>
  </v-container>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  import axios from 'axios';
  import { hostname } from '@/common';
  import '@xterm/xterm/css/xterm.css';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';

  export default {
    mounted() {
      this.fetchData();
    },
    data: () => ({
    }),
    methods: {
      async fetchData() {
        let name = (this.$route.params as { name: string }).name;
        let term = new Terminal({ convertEol: true, scrollback: 1000000 });
        let fitAddon = new FitAddon();
        term.loadAddon(fitAddon);
        term.open(this.$refs.element as HTMLElement);
        fitAddon.fit();
        term.write((await axios.get(`/logs/${name}`)).data);
      }
    }
  }
</script>