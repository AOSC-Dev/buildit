<template>
  <v-container>
    <v-card>
      <v-card-item>
        <v-card-title>Worker #{{ worker.worker_id }}</v-card-title>
        <v-card-subtitle>{{ worker.hostname }}</v-card-subtitle>
      </v-card-item>
      <v-card-text>
        Architecture: {{ worker.arch }}
        <br/>
        Git commit: {{ worker.git_commit }}
        <br/>
        Memory size: {{ worker.memory_bytes !== undefined && prettyBytes(worker.memory_bytes) }}
        <br/>
        Logical cores: {{ worker.logical_cores }}
        <br/>
        Last heartbeat time: {{ worker.last_heartbeat_time }}
        <br/>
        <div v-if="worker.running_job_id !== undefined && worker.running_job_id !== null">
          Running job id: 
          <router-link :to="{ path: `/workers/${worker.running_job_id}` }">
            {{ worker.running_job_id }}
          </router-link>
          <br/>
        </div>
        Built job count: {{ worker.built_job_count }}
      </v-card-text>
    </v-card>
  </v-container>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  import prettyBytes from 'pretty-bytes';
  import axios from 'axios';
  import { hostname } from '@/common';

  interface WorkerInfoResponse {
    worker_id: number;
    hostname: String;
    arch: string;
    git_commit: string;
    memory_bytes: number;
    logical_cores: number;
    last_heartbeat_time: string;
    running_job_id: number;
    built_job_count: number;
  }

  export default {
    mounted() {
      this.fetchData();
    },
    data: () => ({
      worker: {} as WorkerInfoResponse
    }),
    methods: {
      async fetchData() {
        let worker_id = (this.$route.params as { id: string }).id;
        this.worker = (await axios.get(hostname + `/api/worker/info?worker_id=${worker_id}`)).data;
      }
    }
  }
</script>