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
        Creation time: {{ job.creation_time }}{{ job.creation_time !== undefined && job.creation_time !== null ? (", " + new TimeAgo('en-US').format(new Date(job.creation_time))) : "" }}
        <br/>
        Running since: {{ job.assign_time }}{{ job.assign_time !== undefined && job.assign_time !== null ? (", " + new TimeAgo('en-US').format(new Date(job.assign_time))) : "" }}
        <br/>
        Time elapsed: {{ job.elapsed_secs }}
        <br/>
        Finish time: {{ job.finish_time }}{{ job.finish_time !== undefined && job.finish_time !== null ? (", " + new TimeAgo('en-US').format(new Date(job.finish_time))) : "" }}
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
          Running by worker
          <router-link :to="{ path: `/workers/${job.assigned_worker_id}` }">
            #{{ job.assigned_worker_id }}: {{ job.assigned_worker_hostname }}
          </router-link>
          <br/>
        </div>
        <div v-if="job.built_by_worker_id !== null && job.built_by_worker_id !== undefined">
          Built by worker
          <router-link :to="{ path: `/workers/${job.built_by_worker_id}` }">
            #{{ job.built_by_worker_id }}: {{ job.built_by_worker_hostname }}
          </router-link>
          <br/>
        </div>
        <div v-if="job.pushpkg_success === false">
          Failed to push package to repo
          <br/>
        </div>
        <div v-if="job.require_min_core !== undefined && job.require_min_core !== null">
          Requires worker to have at least {{ job.require_min_core }} logical cores to build this job
          <br/>
        </div>
        <div v-if="job.require_min_total_mem !== undefined && job.require_min_total_mem !== null">
          Requires worker to have at least {{ prettyBytes(job.require_min_total_mem, { binary: true }) }} total memory to build this job
          <br/>
        </div>
        <div v-if="job.require_min_total_mem_per_core !== undefined && job.require_min_total_mem_per_core !== null">
          Requires worker to have at least {{ prettyBytes(job.require_min_total_mem_per_core, { binary: true }) }} total memory per logical core to build this job
          <br/>
        </div>
        <div v-if="job.require_min_disk !== undefined && job.require_min_disk !== null">
          Requires worker to have at least {{ prettyBytes(job.require_min_disk) }} free disk space to build this job
          <br/>
        </div>
        <v-btn
          icon="true"
          rounded
          size="x-small"
          v-if="job.status === 'failed'"
          style="margin-top: 5px;margin-bottom: 5px;"
          @click="restartJob(job.job_id)">
          <v-icon>mdi:mdi-restart</v-icon>
          <v-tooltip activator="parent" location="bottom">
            Restart
          </v-tooltip>
        </v-btn>
      </v-card-text>
    </v-card>
    <v-snackbar v-model="jobRestartSnackbar" timeout="2000">
      Job restarted as #{{ newJobID }}
    </v-snackbar>
  </v-container>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  import axios from 'axios';
  import { hostname } from '@/common';
  import prettyBytes from 'pretty-bytes';
  import TimeAgo from 'javascript-time-ago'
  import en from 'javascript-time-ago/locale/en'

  TimeAgo.addDefaultLocale(en)

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
    require_min_core: number;
    require_min_total_mem: number;
    require_min_total_mem_per_core: number;
    require_min_disk: number;
    assign_time: string;

    git_branch: string;
    git_sha: string;
    github_pr: number;

    assigned_worker_hostname: string;
    built_by_worker_hostname: string;
  }

  export default {
    mounted() {
      this.fetchData();
    },
    data: () => ({
      job: {} as JobInfoResponse,
      jobRestartSnackbar: false,
      newJobID: 0,
    }),
    methods: {
      async fetchData() {
        let job_id = (this.$route.params as { id: string }).id;
        this.job = (await axios.get(hostname + `/api/job/info?job_id=${job_id}`)).data;
      },
      async restartJob (id: number) {
        let data = (await axios.post(hostname + `/api/job/restart`, {
          job_id: id,
        })).data;
        this.newJobID = data.job_id;
        this.jobRestartSnackbar = true;
      }
    }
  }
</script>