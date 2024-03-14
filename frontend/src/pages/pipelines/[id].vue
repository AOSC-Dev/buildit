<template>
  <v-container>
    <v-card>
      <v-card-item>
        <v-card-title>Pipeline #{{ pipeline.pipeline_id }}</v-card-title>
      </v-card-item>
      <v-card-text>
        Packages: {{ pipeline.packages }}
        <br/>
        Architectures: {{ pipeline.archs }}
        <br/>
        Creation time: {{ pipeline.creation_time }}
        <br/>
        Git branch: {{ pipeline.git_branch }}
        <br/>
        Git commit: {{ pipeline.git_sha }}
        <br/>
        GitHub pr: {{ pipeline.github_pr }}
        <br/>
        Jobs: <div v-for="job in pipeline.jobs" :key="job.job_id">
          Job
          <router-link :to="{ path: `/jobs/${job.job_id}` }">
            #{{ job.job_id }}
          </router-link>
        </div>
      </v-card-text>
    </v-card>
  </v-container>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  import axios from 'axios';
  import { hostname } from '@/common';

  interface PipelineInfoResponseJob {
    job_id: number;
  }

  interface PipelineInfoResponse {
    pipeline_id: number;
    packages: string;
    archs: string;
    git_branch: string;
    git_sha: string;
    creation_time: string;
    github_pr: number;

    jobs: PipelineInfoResponseJob[];
  }

  export default {
    mounted() {
      this.fetchData();
    },
    data: () => ({
      pipeline: {} as PipelineInfoResponse
    }),
    methods: {
      async fetchData() {
        let pipeline_id = (this.$route.params as { id: string }).id;
        this.pipeline = (await axios.get(hostname + `/api/pipeline/info?pipeline_id=${pipeline_id}`)).data;
      }
    }
  }
</script>