<template>
  <v-container>
    <v-row>
      <v-col>
        <v-data-table-server
          :items-per-page="itemsPerPage"
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
                  <v-icon v-if="(job as Job).status === 'success'">
                    mdi:mdi-check-circle-outline
                  </v-icon>
                  <v-icon v-else-if="(job as Job).status === 'failed'">
                    mdi:mdi-close-circle-outline
                  </v-icon>
                  <v-icon v-else-if="(job as Job).status === 'running'">
                    mdi:mdi-progress-question
                  </v-icon>
                  <v-icon v-else-if="(job as Job).status === 'error'">
                    mdi:mdi-alert-circle-outline
                  </v-icon>
                  <v-icon v-else-if="(job as Job).status === 'created'">
                    mdi:mdi-play-circle-outline
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

  interface LoadItemsOpts {
    page: number;
    itemsPerPage: number;
  }

  interface Job {
    job_id: number;
    arch: string;
    status: string;
    build_success: boolean;
    pushpkg_success: boolean;
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
    data: () => ({
      itemsPerPage: 25,
      headers: [
        { title: 'Status', key: 'status', sortable: false },
        { title: 'Pipeline', key: 'pipeline', sortable: false },
        { title: 'Created by', key: 'created_by', sortable: false },
        { title: 'Jobs', key: 'jobs', sortable: false },
      ],
      loading: true,
      totalItems: 0,
      serverItems: []
    }),
    methods: {
      async loadItems (opts: LoadItemsOpts) {
        this.loading = true;
        let data = (await axios.get(hostname + `/api/pipeline/list?page=${opts.page}&items_per_page=${opts.itemsPerPage}`)).data;
        this.totalItems = data.total_items;
        this.serverItems = data.items;
        this.loading = false;
      }
    }
  }
</script>