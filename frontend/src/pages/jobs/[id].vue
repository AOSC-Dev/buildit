<template>
  <v-container>
    <v-card>
      <v-card-item>
        <v-card-title>Job #{{ job.job_id }}</v-card-title>
        <v-card-subtitle>
          Pipeline 
          <router-link :to="{ path: `/pipelines/${job.pipeline_id}` }">
            #{{ job.pipeline_id }}
          </router-link>
        </v-card-subtitle>
      </v-card-item>
      <v-card-text>
        Creation time: {{ job.creation_time }}
        <br/>
        Finish time: {{ job.finish_time }}
        <br/>
        Time elapsed: {{ job.elapsed_secs }}
        <br/>
        Git commit: <a :href="`https://github.com/AOSC-Dev/aosc-os-abbs/commit/${job.git_sha}`">
          {{ job.git_sha }}
        </a>
        <br/>
        Git branch: <a :href="`https://github.com/AOSC-Dev/aosc-os-abbs/tree/${job.git_branch}`">
          {{ job.git_branch }}
        </a>
        <br/>
        Architecture: {{ job.arch }}
        <br/>
        Package(s) to build: {{ job.packages }}
        <br/>
        Package(s) successfully built: {{ job.successful_packages }}
        <br/>
        Package(s) failed to build: {{ job.failed_package }}
        <br/>
        Package(s) not built due to previous build failure: {{ job.skipped_packages }}
        <br/>
        <div v-if="job.log_url !== null && job.log_url !== undefined">
          Log: <a :href="job.log_url">Raw</a> or
          <router-link :to="{ path: job.log_url.replace('https://buildit.aosc.io/logs/', '/web-logs/') }">
            Web Viewer
          </router-link>
          <br/>
        </div>
        Status: {{ job.status }}
        <div v-if="job.assigned_worker_id !== null && job.assigned_worker_id !== undefined">
          Running by worker #
          <router-link :to="{ path: `/workers/${job.assigned_worker_id}` }">
            {{ job.assigned_worker_id }}
          </router-link>
          <br/>
        </div>
        <div v-if="job.built_by_worker_id !== null && job.built_by_worker_id !== undefined">
          Built by worker #
          <router-link :to="{ path: `/workers/${job.built_by_worker_id}` }">
            {{ job.built_by_worker_id }}
          </router-link>
          <br/>
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
    built_by_worker_id: number;
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
        let job_id = (this.$route.params as { id: string }).id;
        this.job = (await axios.get(hostname + `/api/job/info?job_id=${job_id}`)).data;
      }
    }
  }
</script>