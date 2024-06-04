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
            <br/>
            {{
              (item as Worker).internet_connectivity === false ? "No internet connectivity" : ""
            }}
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
  import prettyBytes from 'pretty-bytes';
  import { hostname } from '@/common';
  import TimeAgo from 'javascript-time-ago'
  import en from 'javascript-time-ago/locale/en'

  TimeAgo.addDefaultLocale(en)

  interface LoadItemsOpts {
    page: number;
    itemsPerPage: number;
  }

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
    data: () => ({
      itemsPerPage: 50,
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
      totalItems: 0,
      serverItems: []
    }),
    methods: {
      async loadItems (opts: LoadItemsOpts) {
        this.loading = true;
        let data = (await axios.get(hostname + `/api/worker/list?page=${opts.page}&items_per_page=${opts.itemsPerPage}`)).data;
        this.totalItems = data.total_items;
        this.serverItems = data.items;
        this.loading = false;
      }
    }
  }
</script>