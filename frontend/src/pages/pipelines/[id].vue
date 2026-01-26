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
        Git branch: <a :href="`https://github.com/AOSC-Dev/aosc-os-abbs/tree/${pipeline.git_branch}`">
          {{ pipeline.git_branch }}
        </a>
        <br/>
        Git commit: <a :href="`https://github.com/AOSC-Dev/aosc-os-abbs/commit/${pipeline.git_sha}`">
          {{ pipeline.git_sha }}
        </a>
        <br/>
        <div v-if="pipeline.github_pr !== null && pipeline.github_pr !== undefined">
          GitHub PR: <a :href="`https://github.com/AOSC-Dev/aosc-os-abbs/pull/${pipeline.github_pr}`">
            {{ pipeline.github_pr }}
          </a>
          <br/>
        </div>
        Jobs:
        <div v-for="job in pipeline.jobs" :key="job.job_id">
          <div class="d-inline-flex align-center ga-2">
            <JobStatusIconLink :job-id="job.job_id" :status="job.status" :arch="job.arch" />
            <span>
              Job
              <router-link :to="{ path: `/jobs/${job.job_id}` }">
                #{{ job.job_id }}
              </router-link>
              for {{ job.arch }}
            </span>
          </div>
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
  import JobStatusIconLink from '@/components/JobStatusIconLink.vue';

  interface PipelineInfoResponseJob {
    job_id: number;
    arch: string;
    status: string;
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
