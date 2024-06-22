<template>
  <v-container style="height: 100%; font-family: monospace; background-color: black; color: white;">
    <div v-for="line in lines" v-html="line"></div>
  </v-container>
</template>

<script lang="ts">
import { AnsiUp } from 'ansi_up';
export default {
  mounted() {
    this.fetchData();
  },
  unmounted() {
    this.socket?.close();
    this.socket = undefined;
  },
  data: () => ({
    lines: [] as string[],
    socket: undefined as WebSocket | undefined,
  }),
  methods: {
    fetchData() {
      let name = (this.$route.params as { hostname: string }).hostname;
      this.socket = new WebSocket(
        `wss://buildit.aosc.io/api/ws/viewer/${name}`
      );
      let ansi_up = new AnsiUp();
      this.socket.onmessage = (event) => {
        if (this.lines.length > 5000) {
          this.lines = this.lines.slice(0, 2500);
        }
        this.lines.push(ansi_up.ansi_to_html(event.data) + " <br/> ");
        setTimeout(() => {
          window.scrollTo(0, document.body.scrollHeight);
        }, 100);
      };
      this.socket.onclose = (event) => {
        // reconnect after 1s
        setTimeout(() => {
          this.fetchData();
        }, 1000);
      }
    },
  },
};
</script>
