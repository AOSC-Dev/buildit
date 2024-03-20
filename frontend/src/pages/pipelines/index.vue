<template>
  <v-container>
    <v-row>
      <v-col>
        <v-expansion-panels>
          <v-expansion-panel title="Pipelines">
            <v-expansion-panel-text>
              <v-container class="pa-0">
                <v-row class="pa-0">
                  <v-col class="pa-0">
                    <v-checkbox
                      v-model="stableOnly"
                      label="Stable Branch Only"
                      @update:model-value="loadItems"
                      :hide-details="true">
                    </v-checkbox>
                  </v-col>
                  <v-col class="pa-0">
                    <v-checkbox
                      v-model="githubPROnly"
                      label="GitHub PR Only"
                      @update:model-value="loadItems"
                      :hide-details="true">
                    </v-checkbox>
                  </v-col>
                  <v-col class="pa-0">
                    <v-checkbox
                      v-model="autoRefresh"
                      label="Auto Refresh"
                      :hide-details="true">
                    </v-checkbox>
                  </v-col>
                  <v-spacer></v-spacer>
                  <v-col class="d-flex align-end justify-end">
                    <v-progress-circular
                      :model-value="countdown * 10 / 1"
                      size="36"
                      color="blue"
                      style="margin-right: 20px;"
                      v-if="autoRefresh">
                      <template v-slot:default> {{ countdown }}s </template>
                    </v-progress-circular>
                    <v-btn @click="loadItems">Refresh</v-btn>
                  </v-col>
                </v-row>
              </v-container>
            </v-expansion-panel-text>
          </v-expansion-panel>
        </v-expansion-panels>
      </v-col>
    </v-row>
    <v-row>
      <v-col>
        <v-data-table-server
          v-model:items-per-page="itemsPerPage"
          v-model:page="page"
          :headers="headers"
          :items="serverItems"
          :items-length="totalItems"
          :loading="loading"
          item-value="id"
          @update:options="loadItems">
          <template #item.status="{ item }">
            <div style="margin-top: 5px"></div>
            <v-chip
              color="green"
              variant="flat"
              density="comfortable"
              v-if="(item as Job).status === 'success'"
              prepend-icon="mdi:mdi-check-circle"
              :to="{ path: `/pipelines/${(item as Pipeline).id}` }"
              >
              Passed
            </v-chip>
            <v-chip
              color="red"
              variant="flat"
              density="comfortable"
              v-else-if="(item as Job).status === 'failed'"
              prepend-icon="mdi:mdi-close-circle"
              :to="{ path: `/pipelines/${(item as Pipeline).id}` }"
              >
              Failed
            </v-chip>
            <v-chip
              color="grey"
              variant="flat"
              density="comfortable"
              v-else-if="(item as Job).status === 'running'"
              prepend-icon="mdi:mdi-progress-question"
              :to="{ path: `/pipelines/${(item as Pipeline).id}` }"
              >
              Running
            </v-chip>
            <v-chip
              color="red"
              variant="flat"
              density="comfortable"
              v-else-if="(item as Job).status === 'error'"
              prepend-icon="mdi:mdi-alert-circle"
              :to="{ path: `/pipelines/${(item as Pipeline).id}` }"
              >
              Error
            </v-chip>

            <div class="d-flex align-center">
              <v-icon size="x-small" style="margin-right: 5px;">mdi:mdi-calendar</v-icon>
              <div>
                {{ new TimeAgo('en-US').format(new Date((item as Pipeline).creation_time)) }}
                <v-tooltip activator="parent" location="bottom">
                  {{ new Date((item as Pipeline).creation_time) }}
                </v-tooltip>
              </div>
            </div>
          </template>
          <template #item.pipeline="{ item }">
            <router-link :to="{ path: `/pipelines/${(item as Pipeline).id}` }">
              #{{ (item as Pipeline).id }}: {{ (item as Pipeline).packages }}
            </router-link>
            <br style="margin-bottom: 5px;"/>
            <v-chip
              label
              density="comfortable"
              prepend-icon="mdi:mdi-source-branch"
              :href="`https://github.com/AOSC-Dev/aosc-os-abbs/tree/${(item as Pipeline).git_branch}`"
              style="margin-right: 5px; margin-bottom: 5px;"
              >
              {{ (item as Pipeline).git_branch }}
            </v-chip>
            <v-chip
              label
              density="comfortable"
              prepend-icon="mdi:mdi-source-commit"
              :href="`https://github.com/AOSC-Dev/aosc-os-abbs/commit/${(item as Pipeline).git_sha}`"
              style="margin-right: 5px; margin-bottom: 5px;"
              >
              {{ (item as Pipeline).git_sha.substring(0, 8) }}
            </v-chip>
            <v-chip
              label
              density="comfortable"
              prepend-icon="mdi:mdi-source-pull"
              :href="`https://github.com/AOSC-Dev/aosc-os-abbs/pull/${(item as Pipeline).github_pr}`"
              v-if="(item as Pipeline).github_pr"
              style="margin-right: 5px; margin-bottom: 5px;"
              >
              #{{ (item as Pipeline).github_pr }}
            </v-chip>
          </template>
          <template #item.created_by="{ item }">
            <div v-if="(item as Pipeline).creator_github_login !== null && (item as Pipeline).creator_github_login !== undefined">
              <a
                :href="`https://github.com/${(item as Pipeline).creator_github_login}`">
                <v-avatar
                  size="x-small">
                  <v-img
                    :src="(item as Pipeline).creator_github_avatar_url"
                  ></v-img>
                </v-avatar>
              </a>
            </div>
          </template>
          <template #item.jobs="{ item }">
            <div class="d-inline-flex">
              <div v-for="job in (item as Pipeline).jobs" :link="job.job_id">
                <!-- https://stackoverflow.com/questions/44808474/vue-router-how-to-remove-underline-from-router-link -->
                <router-link
                  style="text-decoration: none; color: inherit;"
                  :to="{ path: `/jobs/${(job as Job).job_id}` }">
                  <v-icon v-if="(job as Job).status === 'success'" color="green" size="large">
                    mdi:mdi-check-circle-outline
                  </v-icon>
                  <v-icon v-else-if="(job as Job).status === 'failed'" color="red" size="large">
                    mdi:mdi-close-circle-outline
                  </v-icon>
                  <v-icon v-else-if="(job as Job).status === 'running'" color="blue" size="large">
                    mdi:mdi-circle-slice-5
                  </v-icon>
                  <v-icon v-else-if="(job as Job).status === 'error'" color="red" size="large">
                    mdi:mdi-alert-circle-outline
                  </v-icon>
                  <v-icon v-else-if="(job as Job).status === 'created'" color="grey" size="large">
                    mdi:mdi-circle-slice-8
                  </v-icon>
                  <v-tooltip activator="parent" location="bottom">
                    Job #{{ (job as Job).job_id }} for {{ (job as Job).arch }}: {{ (job as Job).status }}
                  </v-tooltip>
                </router-link>
              </div>
            </div>
          </template>
        </v-data-table-server>
      </v-col>
    </v-row>
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

  interface Job {
    job_id: number;
    arch: string;
    status: string;
  }

  interface Pipeline {
    id: number;
    git_branch: string;
    git_sha: string;
    creation_time: string;
    github_pr: number;
    packages: string;
    archs: string;
    creator_github_login: string;
    creator_github_avatar_url: string;
    status: string;
    jobs: Job[];
  }

  export default {
    data() {
      return {
        page: Number(this.$route.query.page) || 1,
        itemsPerPage: Number(this.$route.query.items_per_page) || 25,
        stableOnly: this.$route.query.stable_only === "true",
        githubPROnly: this.$route.query.github_pr_only === "true",
        headers: [
          { title: 'Status', key: 'status', sortable: false },
          { title: 'Pipeline', key: 'pipeline', sortable: false },
          { title: 'Created by', key: 'created_by', sortable: false },
          { title: 'Jobs', key: 'jobs', sortable: false },
        ],
        loading: true,
        totalItems: 999999,
        serverItems: [],
        autoRefresh: this.$route.query.auto_refresh === "true",
        intervalHandle: null as any,
        countdown: 0,
      };
    },
    watch: {
      autoRefresh(newValue) {
        if (newValue) {
          this.startAutoRefresh();
        } else {
          clearInterval(this.intervalHandle);
        }

        this.$router.push({path: this.$route.path, query: {
          page: String(this.page),
          items_per_page: String(this.itemsPerPage),
          stable_only: String(this.stableOnly),
          github_pr_only: String(this.githubPROnly),
          auto_refresh: String(this.autoRefresh)
        } });
      }
    },
    methods: {
      startAutoRefresh() {
        this.countdown = 10;
        this.intervalHandle = setInterval(() => {
          if (this.countdown == 0) {
            clearInterval(this.intervalHandle);
            this.loadItems();
          } else {
            this.countdown = this.countdown - 1;
          }
        }, 1000);
      },
      async loadItems () {
        this.$router.push({path: this.$route.path, query: {
          page: String(this.page),
          items_per_page: String(this.itemsPerPage),
          stable_only: String(this.stableOnly),
          github_pr_only: String(this.githubPROnly),
          auto_refresh: String(this.autoRefresh)
        } });

        this.loading = true;
        let data = (await axios.get(hostname + `/api/pipeline/list?page=${this.page}&items_per_page=${this.itemsPerPage}&stable_only=${this.stableOnly}&github_pr_only=${this.githubPROnly}`)).data;
        this.totalItems = data.total_items;
        this.serverItems = data.items;
        this.loading = false;

        if (this.autoRefresh) {
          this.startAutoRefresh();
        }
      }
    }
  }
</script>