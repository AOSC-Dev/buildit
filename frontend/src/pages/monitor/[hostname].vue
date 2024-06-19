<template>
  <v-container style="height: 100%; font-family: monospace">
    <div v-for="line in lines" v-html="line"></div>
  </v-container>
</template>

<script lang="ts">
import { AnsiUp } from 'ansi_up';
export default {
  mounted() {
    this.fetchData();
  },
  data: () => ({
    lines: [] as string[]
  }),
  methods: {
    fetchData() {
      let name = (this.$route.params as { hostname: string }).hostname;
      const socket = new WebSocket(
        `wss://buildit.aosc.io/api/ws/viewer/${name}`
      );
      socket.onmessage = (event) => {
        if (this.lines.length > 5000) {
          this.lines = [];
        }
        const data = event.data;
        let ansi_up = new AnsiUp();
        this.lines.push(ansi_up.ansi_to_html(data) + " <br/> ");
        window.scrollTo(0, document.body.scrollHeight);
      };
    },
  },
};
</script>
