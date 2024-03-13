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
          <template #item.id="{ item }">
            <router-link :to="{ path: `/workers/${(item as Worker).id}` }">
              {{ (item as Worker).id }}
            </router-link>
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

  interface LoadItemsOpts {
    page: number;
    itemsPerPage: number;
  }

  interface Worker {
    id: number;
  }
  export default {
    data: () => ({
      itemsPerPage: 10,
      headers: [
        { title: 'Worker ID', key: 'id', sortable: false },
        { title: 'Hostname', key: 'hostname', sortable: false },
        { title: 'Architecture', key: 'arch', sortable: false },
        { title: 'Logical Cores', key: 'logical_cores', sortable: false },
        { title: 'Memory Size', key: 'memory_bytes', sortable: false, value: (item: any) => prettyBytes(item.memory_bytes) },
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