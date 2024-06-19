<template>
  <v-container style="height: 100%; font-family: monospace">
    <div v-html="html"></div>
  </v-container>
</template>

<script lang="ts">
import { AnsiUp } from 'ansi_up';
export default {
  mounted() {
    this.fetchData();
  },
  data: () => ({
    html: "",
    lines: 0,
  }),
  methods: {
    fetchData() {
      let name = (this.$route.params as { hostname: string }).hostname;
      const socket = new WebSocket(
        `wss://buildit.aosc.io/api/ws/viewer/${name}`
      );
      socket.onmessage = (event) => {
        if (this.lines > 5000) {
          this.html = "";
          this.lines = 0;
        }
        const data = event.data;
        let ansi_up = new AnsiUp();
        this.html += ansi_up.ansi_to_html(data) + " <br/> ";
        this.line += 1;
        window.scrollTo(0, document.body.scrollHeight);
      };
    },
  },
};
</script>
