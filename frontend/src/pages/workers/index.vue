<template>
  <v-container>
    <v-row>
      <v-col>
        <v-expansion-panels>
          <v-expansion-panel title="Workers">
            <v-expansion-panel-text>
              <v-container class="pa-0">
                <v-row class="pa-0">
                  <v-col class="pa-0">
                    <v-select
                      v-model="statuses"
                      :items="statusItems"
                      label="Status"
                      multiple
                      chips
                      closable-chips
                      density="compact"
                      :hide-details="true">
                    </v-select>
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
        <v-data-table
          :items-per-page="-1"
          :headers="headers"
          :items="filteredItems"
          :loading="loading"
          item-value="id"
          hide-default-footer>
          <template #item.hostname="{ item }">
            <router-link :to="{ path: `/workers/${(item as Worker).id}` }">
              {{ (item as Worker).hostname }}
            </router-link>
          </template>
          <template #item.status="{ item }">
            <v-chip
              color="green"
              variant="flat"
              density="comfortable"
              v-if="(item as Worker).is_live"
              prepend-icon="mdi:mdi-check-circle"
              style="margin-top: 5px; margin-bottom: 3px;"
              >
              Live
            </v-chip>
            <v-chip
              color="red"
              variant="flat"
              density="comfortable"
              v-else
              prepend-icon="mdi:mdi-close-circle"
              style="margin-top: 5px; margin-bottom: 3px;"
              >
              Dead
            </v-chip>
            <v-btn
              icon="true"
              rounded
              size="x-small"
              v-if="(item as Worker).is_live"
              :to="{ path: '/monitor/' + (item as Worker).hostname }"
              style="margin-left: 5px;margin-bottom: 5px;">
              <v-icon>mdi:mdi-receipt-text</v-icon>
              <v-tooltip activator="parent" location="bottom">
                Monitor
              </v-tooltip>
            </v-btn>
            <br/>
            Last seen {{ new TimeAgo('en-US').format(new Date((item as Worker).last_heartbeat_time)) }}
            <div v-if="(item as Worker).running_job_id !== null && (item as Worker).running_job_id !== undefined">
              Running job
              <router-link :to="{ path: `/jobs/${(item as Worker).running_job_id}` }">
                # {{ (item as Worker).running_job_id }}
              </router-link>
              {{
                (item as Worker).running_job_assign_time !== null && (item as Worker).running_job_assign_time !== undefined ?
                  " since " + new TimeAgo('en-US').format(new Date((item as Worker).running_job_assign_time)) : ""
              }}
            </div>
            <div v-if="(item as Worker).internet_connectivity === false">
              No internet connectivity
            </div>
          </template>
        </v-data-table>
      </v-col>
    </v-row>
  </v-container>
</template>

<script lang="ts" setup>
  //
</script>

<script lang="ts">
  import axios from 'axios';
  import prettyBytes from 'pretty-bytes';
  import { hostname } from '@/common';
  import TimeAgo from 'javascript-time-ago'
  import en from 'javascript-time-ago/locale/en'

  TimeAgo.addDefaultLocale(en)

  interface Worker {
    id: number;
    hostname: string;
    is_live: boolean;
    last_heartbeat_time: string;
    logical_cores: number;
    memory_bytes: number;
    disk_free_space_bytes: number;
    running_job_id: number;
    running_job_assign_time: string;
    internet_connectivity: boolean;
  }

  export default {
    data() {
      return {
        statuses: [] as string[],
        statusItems: [
          { title: 'Live', value: 'live' },
          { title: 'Dead', value: 'dead' },
          { title: 'Idle', value: 'idle' },
          { title: 'Busy', value: 'busy' },
        ],
        headers: [
          { title: 'Hostname', key: 'hostname', sortable: false },
          { title: 'Architecture', key: 'arch', sortable: false },
          { title: 'Logical Cores', key: 'logical_cores', sortable: false },
          { title: 'Memory Size', key: 'memory_bytes', sortable: false, value: (item: any) => prettyBytes(item.memory_bytes, { binary: true }) },
          { title: 'Memory Per Core', key: 'memory_per_core', sortable: false, value: (item: any) => prettyBytes(item.memory_bytes / item.logical_cores, { binary: true }) },
          { title: 'Disk Free Space Size', key: 'disk_free_space_bytes', sortable: false, value: (item: any) => prettyBytes(item.disk_free_space_bytes) },
          { title: 'Status', key: 'status', sortable: false },
        ],
        loading: true,
        serverItems: [] as Worker[]
      };
    },
    computed: {
      filteredItems(): Worker[] {
        if (this.statuses.length === 0) {
          return this.serverItems;
        }
        return this.serverItems.filter((worker: Worker) => {
          const status = this.workerStatus(worker);
          // "live" matches any live worker, whether idle or busy
          return this.statuses.includes(status) || (this.statuses.includes('live') && status !== 'dead');
        });
      }
    },
    mounted() {
      this.loadItems();
    },
    methods: {
      workerStatus(worker: Worker): string {
        if (!worker.is_live) {
          return 'dead';
        }
        return worker.running_job_id !== null && worker.running_job_id !== undefined ? 'busy' : 'idle';
      },
      async loadItems () {
        this.loading = true;
        let data = (await axios.get(hostname + `/api/worker/list?page=1&items_per_page=-1`)).data;
        this.serverItems = data.items;
        this.loading = false;
      }
    }
  }
</script>
