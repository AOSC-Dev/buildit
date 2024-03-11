<template>
  <v-container>
    <v-row>
      <v-col>
        <v-card height="180">
          <v-card-item>
            <v-card-title>Pipelines</v-card-title>
          </v-card-item>
          <v-card-text>
            Total: {{status.total_pipeline_count}}
          </v-card-text>
        </v-card>
      </v-col>
      <v-col>
        <v-card height="180">
          <v-card-item>
            <v-card-title>Jobs</v-card-title>
          </v-card-item>
          <v-card-text>
            Total: {{status.total_job_count}}
            <p></p>
            Pending: {{status.pending_job_count}}
            <p></p>
            Running: {{status.running_job_count}}
            <p></p>
            Finished: {{status.finished_job_count}}
          </v-card-text>
        </v-card>
      </v-col>
      <v-col>
        <v-card height="180">
          <v-card-item>
            <v-card-title>Workers</v-card-title>
          </v-card-item>
          <v-card-text>
            Total: {{status.total_worker_count}}
            <p></p>
            Live: {{status.live_worker_count}}
            <p></p>
            Total Logical Cores: {{status.total_logical_cores}}
            <p></p>
            Total Memory: {{status.total_memory_bytes && prettyBytes(Number(status.total_memory_bytes))}}
            <p></p>
          </v-card-text>
        </v-card>
      </v-col>
    </v-row>
    <v-row>
      <v-col v-for="arch in archs" :link="arch" cols="3">
        <v-card height="200">
          <v-card-item>
            <v-card-title>{{ arch }}</v-card-title>
          </v-card-item>
          <v-card-text>
            Total Workers: {{status.by_arch && status.by_arch[arch].total_worker_count}}
            <p></p>
            Live Workers: {{status.by_arch && status.by_arch[arch].live_worker_count}}
            <p></p>
            Total Logical Cores: {{status.by_arch && status.by_arch[arch].total_logical_cores}}
            <p></p>
            Total Memory: {{status.by_arch && prettyBytes(Number(status.by_arch[arch].total_memory_bytes))}}
            <p></p>
            Total Jobs: {{status.by_arch && status.by_arch[arch].total_job_count}}
            <p></p>
            Pending Jobs: {{status.by_arch && status.by_arch[arch].pending_job_count}}
            <p></p>
            Running Jobs: {{status.by_arch && status.by_arch[arch].running_job_count}}
          </v-card-text>
        </v-card>
      </v-col>
    </v-row>
  </v-container>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  import prettyBytes from 'pretty-bytes';
  import axios from 'axios';
  import { hostname } from '@/common';
  interface DashboardStatusResponseByArch {
    total_worker_count: number;
    live_worker_count: number;
    total_logical_cores: number;
    total_memory_bytes: string;

    total_job_count: number;
    pending_job_count: number;
    running_job_count: number;
  }

  interface DashboardStatusResponse {
    total_pipeline_count: number;

    total_job_count: number;
    pending_job_count: number;
    running_job_count: number;
    finished_job_count: number;

    total_worker_count: number;
    live_worker_count: number;
    total_logical_cores: number;
    total_memory_bytes: string;

    by_arch: { [key:string]: DashboardStatusResponseByArch };
  }

  export default {
    mounted() {
      this.fetchData();
    },
    methods: {
      async fetchData() {
        this.status = (await axios.get(hostname + "/api/dashboard/status")).data as DashboardStatusResponse;
      }
    },
    data: () => ({
      status: {} as DashboardStatusResponse,
      archs: [
        "amd64",
        "arm64",
        "loongarch64",
        "loongson3",
        "mips64r6el",
        "ppc64el",
        "riscv64"
      ]
    }),
  }
</script>
