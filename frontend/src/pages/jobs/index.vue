<template>
  <v-container>
    <v-row>
      <v-col>
        <v-data-table-server
          :page="page"
          :items-per-page="itemsPerPage"
          :headers="headers"
          :items="serverItems"
          :items-length="totalItems"
          :loading="loading"
          item-value="id"
          @update:options="loadItems">
          <template #item.status="{ item }">
            <div style="margin-top: 20px"></div>
            <v-chip
              color="green"
              variant="flat"
              density="comfortable"
              v-if="(item as Job).status === 'finished' && (item as Job).build_success"
              prepend-icon="mdi:mdi-check-circle"
              :to="{ path: `/jobs/${(item as Job).id}` }"
              >
              Passed
            </v-chip>
            <v-chip
              color="red"
              variant="flat"
              density="comfortable"
              v-else-if="(item as Job).status === 'finished' && !(item as Job).build_success"
              prepend-icon="mdi:mdi-close-circle"
              :to="{ path: `/jobs/${(item as Job).id}` }"
              >
              Failed
            </v-chip>
            <v-chip
              color="grey"
              variant="flat"
              density="comfortable"
              v-else-if="(item as Job).status === 'assigned'"
              prepend-icon="mdi:mdi-progress-question"
              :to="{ path: `/jobs/${(item as Job).id}` }"
              >
              Running
            </v-chip>
            <v-chip
              color="red"
              variant="flat"
              density="comfortable"
              v-else-if="(item as Job).status === 'error'"
              prepend-icon="mdi:mdi-alert-circle"
              :to="{ path: `/jobs/${(item as Job).id}` }"
              >
              Error
            </v-chip>
            <v-chip
              color="grey"
              variant="flat"
              density="comfortable"
              v-else="(item as Job).status === 'created'"
              prepend-icon="mdi:mdi-play-circle"
              :to="{ path: `/jobs/${(item as Job).id}` }"
              >
              Created
            </v-chip>
            <div
              class="d-flex align-center">
              <v-icon size="x-small" style="margin-right: 5px;">mdi:mdi-clock-outline</v-icon>
              <!-- https://stackoverflow.com/a/1322771/2148614 -->
              {{ new Date((item as Job).elapsed_secs * 1000).toISOString().substring(11, 19)  }}
            </div>
            <div class="d-flex align-center">
              <v-icon size="x-small" style="margin-right: 5px;">mdi:mdi-calendar</v-icon>
              <div>
                {{ new TimeAgo('en-US').format(new Date((item as Job).creation_time)) }}
                <v-tooltip activator="parent" location="bottom">
                  {{ new Date((item as Job).creation_time) }}
                </v-tooltip>
              </div>
            </div>
            <div style="margin-bottom: 10px"></div>
          </template>
          <template #item.job="{ item }">
            <router-link :to="{ path: `/jobs/${(item as Job).id}` }">
              #{{ (item as Job).id }}: {{ (item as Job).packages }}
            </router-link>
            <br style="margin-bottom: 5px;"/>
            <v-chip
              label
              density="comfortable"
              prepend-icon="mdi:mdi-source-branch"
              :href="`https://github.com/AOSC-Dev/aosc-os-abbs/tree/${(item as Job).git_branch}`"
              style="margin-right: 5px; margin-bottom: 5px;"
              >
              {{ (item as Job).git_branch }}
            </v-chip>
            <v-chip
              label
              density="comfortable"
              prepend-icon="mdi:mdi-source-commit"
              :href="`https://github.com/AOSC-Dev/aosc-os-abbs/commit/${(item as Job).git_sha}`"
              style="margin-right: 5px; margin-bottom: 5px;"
              >
              {{ (item as Job).git_sha.substring(0, 8) }}
            </v-chip>
            <v-chip
              label
              density="comfortable"
              prepend-icon="mdi:mdi-source-pull"
              :href="`https://github.com/AOSC-Dev/aosc-os-abbs/pull/${(item as Job).github_pr}`"
              v-if="(item as Job).github_pr"
              style="margin-right: 5px; margin-bottom: 5px;"
              >
              #{{ (item as Job).github_pr }}
            </v-chip>
            <v-chip
              label
              density="comfortable"
              prepend-icon="mdi:mdi-cpu-64-bit"
              style="margin-right: 5px; margin-bottom: 5px;"
              >
              {{ (item as Job).arch }}
            </v-chip>
          </template>
          <template #item.pipeline="{ item }">
            <router-link :to="{ path: `/pipelines/${(item as Job).pipeline_id}` }">
              #{{ (item as Job).pipeline_id }}
            </router-link>
          </template>
          <template #item.actions="{ item }">
            <v-btn
              icon="true"
              rounded
              size="x-small"
              v-if="(item as Job).log_url !== null && (item as Job).log_url !== undefined"
              :to="{ path: (item as Job).log_url.replace('https://buildit.aosc.io/logs/', '/web-logs/') }"
              style="margin-right: 5px;margin-bottom: 5px;">
              <v-icon>mdi:mdi-history</v-icon>
              <v-tooltip activator="parent" location="bottom">
                View Log
              </v-tooltip>
            </v-btn>
            <v-btn
              icon="true"
              rounded
              size="x-small"
              v-if="(item as Job).status === 'finished'"
              style="margin-right: 5px;margin-bottom: 5px;"
              @click="restartJob((item as Job).id)">
              <v-icon>mdi:mdi-restart</v-icon>
              <v-tooltip activator="parent" location="bottom">
                Restart
              </v-tooltip>
            </v-btn>
          </template>
        </v-data-table-server>
      </v-col>
    </v-row>
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
  import TimeAgo from 'javascript-time-ago'
  import en from 'javascript-time-ago/locale/en'

  TimeAgo.addDefaultLocale(en)

  interface LoadItemsOpts {
    page: number;
    itemsPerPage: number;
  }

  interface Job {
    id: number;
    pipeline_id: number;
    build_success: boolean;
    status: string;
    packages: string;
    arch: string;
    git_branch: string;
    git_sha: string;
    github_pr: number;
    log_url: string;
    elapsed_secs: number;
    creation_time: string;
  }

  export default {
    data: () => ({
      page: 1,
      itemsPerPage: 25,
      headers: [
        { title: 'Status', key: 'status', sortable: false },
        { title: 'Job', key: 'job', sortable: false },
        { title: 'Pipeline', key: 'pipeline', sortable: false },
        { title: 'Actions', key: 'actions', sortable: false },
      ],
      loading: true,
      totalItems: 0,
      serverItems: [],
      jobRestartSnackbar: false,
      newJobID: 0,
    }),
    methods: {
      async loadItems () {
        this.loading = true;
        let url = hostname + `/api/job/list?page=${this.page}&items_per_page=${this.itemsPerPage}`;
        let data = (await axios.get(url)).data;
        this.totalItems = data.total_items;
        this.serverItems = data.items;
        this.loading = false;
      },
      async restartJob (id: number) {
        let data = (await axios.post(hostname + `/api/job/restart`, {
          job_id: id,
        })).data;
        this.newJobID = data.job_id;
        this.jobRestartSnackbar = true;
        await this.loadItems();
      }
    }
  }
</script>