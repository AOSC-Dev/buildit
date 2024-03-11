<template>
  <v-container>
    <v-card>
      <v-card-item>
        <v-card-title>Job #{{ job.job_id }}</v-card-title>
        <v-card-subtitle>Pipeline #{{ job.pipeline_id }}</v-card-subtitle>
      </v-card-item>
      <v-card-text>
        Creation time: {{ job.creation_time }}
        <p></p>
        Finish time: {{ job.finish_time }}
        <p></p>
        Time elapsed: {{ job.elapsed_secs }}
        <p></p>
        Git commit: {{ job.git_sha }}
        <p></p>
        Git branch: {{ job.git_branch }}
        <p></p>
        Architecture: {{ job.arch }}
        <p></p>
        Package(s) to build: {{ job.packages }}
        <p></p>
        Package(s) successfully built: {{ job.successful_packages }}
        <p></p>
        Package(s) failed to build: {{ job.failed_package }}
        <p></p>
        Package(s) not built due to previous build failure: {{ job.skipped_packages }}
        <p></p>
        Log: {{ job.log_url }}
        <p></p>
        Status: {{ job.status }}
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

  interface JobInfoResponse {
    job_id: number;
    pipeline_id: number;
    packages: string;
    arch: string;
    creation_time: string;
    status: string;
    build_success: boolean;
    pushpkg_success: boolean;
    successful_packages: string;
    failed_package: string;
    skipped_packages: string;
    log_url: string;
    finish_time: string;
    error_message: string;
    elapsed_secs: number;
    assigned_worker_id: number;
    git_branch: string;
    git_sha: string;
    github_pr: number;
  }

  export default {
    mounted() {
      this.fetchData();
    },
    data: () => ({
      job: {} as JobInfoResponse
    }),
    methods: {
      async fetchData() {
        let job_id = this.$route.params.id;
        this.job = (await axios.get(hostname + `/api/job/info?job_id=${job_id}`)).data;
      }
    }
  }
</script>