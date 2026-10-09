<template>
  <v-container>
    <h1 style="margin-block: 0.3em;">AOSC BuildIt!</h1>
    <v-row>
      <v-col cols="6" sm="4">
        <v-card style="min-height: 100%">
          <v-card-item>
            <v-card-title>Pipelines</v-card-title>
          </v-card-item>
          <v-card-text>
            Total: {{status.total_pipeline_count}}
          </v-card-text>
        </v-card>
      </v-col>
      <v-col cols="6" sm="4">
        <v-card style="min-height: 100%">
          <v-card-item>
            <v-card-title>Jobs</v-card-title>
          </v-card-item>
          <v-card-text>
            Total: {{status.total_job_count}}
            <br/>
            Pending: {{status.pending_job_count}}
            <br/>
            Running: {{status.running_job_count}}
            <br/>
            Finished: {{status.finished_job_count}}
          </v-card-text>
        </v-card>
      </v-col>
      <v-col cols="6" sm="4">
        <v-card style="min-height: 100%">
          <v-card-item>
            <v-card-title>Workers</v-card-title>
          </v-card-item>
          <v-card-text>
            Total: {{status.total_worker_count}}
            <br/>
            Live: {{status.live_worker_count}}
            <br/>
            Total Logical Cores: {{status.total_logical_cores}}
            <br/>
            Total Memory: {{status.total_memory_bytes && prettyBytes(Number(status.total_memory_bytes), { binary: true })}}
            <br/>
          </v-card-text>
        </v-card>
      </v-col>
    </v-row>
    
    <template v-for="[name, arch_group] in Object.entries(arch_groups)" :key="name">
      <v-divider style="margin-top: 1.5em;" />

      <h2 style="margin-block: 0.5em; display: flex; align-items: baseline;">
        <span>{{ arch_group.name }}</span>
        <span style="font-size: small; font-weight: normal; margin-left: 0.5em; cursor: help;">
          (What is it?)
          <v-tooltip interactive activator="parent">
            {{ arch_group.description }}
          </v-tooltip>
        </span>
      </h2>

      <v-row>
        <v-col v-for="arch in arch_group.archs" :key="arch" :link="arch" cols="6" sm="4" md="3">
          <v-card style="min-height: 100%">
            <v-card-item>
              <v-card-title>{{ arch }}</v-card-title>
            </v-card-item>
            <v-card-text>
              Total Workers: {{status.by_arch && status.by_arch[arch].total_worker_count}}
              <br/>
              Live Workers: {{status.by_arch && status.by_arch[arch].live_worker_count}}
              <br/>
              Total Logical Cores: {{status.by_arch && status.by_arch[arch].total_logical_cores}}
              <br/>
              Total Memory: {{status.by_arch && prettyBytes(Number(status.by_arch[arch].total_memory_bytes), { binary: true })}}
              <br/>
              Total Jobs: {{status.by_arch && status.by_arch[arch].total_job_count}}
              <br/>
              Pending Jobs: {{status.by_arch && status.by_arch[arch].pending_job_count}}
              <br/>
              Running Jobs: {{status.by_arch && status.by_arch[arch].running_job_count}}
            </v-card-text>
          </v-card>
        </v-col>
      </v-row>
    </template>
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
      arch_groups: {
        mainline: {
          name: "Mainline",
          description: "It's mainline",
          archs: [
            "amd64",
            "arm64",
            "loongarch64",
            "loongarch64_nosimd",
            "ppc64el",
            "riscv64",
            "loongson3"
          ]
        },
        retro: {
          name: "Retro",
          description: "It's retro",
          archs: [
            "alpha",
            "armv4",
            "armv5te",
            "armv6hf",
            "armv7hf",
            "i486",
            "loongson2f",
            "powerpc",
            "ppc64",
          ]
        },
      },
    }),
  }
</script>
