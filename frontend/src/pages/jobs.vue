<template>
  <v-container>
    <v-row>
      <v-col>
        <v-data-table-server
          v-model:items-per-page="itemsPerPage"
          :headers="headers"
          :items="serverItems"
          :items-length="totalItems"
          :loading="loading"
          :sortable="false"
          item-value="id"
          @update:options="loadItems">

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

  interface LoadItemsOpts {
    page: number;
    itemsPerPage: number;
  }

  export default {
    data: () => ({
      itemsPerPage: 10,
      headers: [
        { title: 'Job ID', key: 'id' },
        { title: 'Pipeline ID', key: 'pipeline_id' },
        { title: 'Packages', key: 'packages' },
        { title: 'Architecture', key: 'arch' },
      ],
      loading: true,
      totalItems: 0,
      serverItems: []
    }),
    methods: {
      async loadItems (opts: LoadItemsOpts) {
        this.loading = true;
        let data = (await axios.get(hostname + `/api/job/list?page=${opts.page}&items_per_page=${opts.itemsPerPage}`)).data;
        this.totalItems = data.total_items;
        this.serverItems = data.items;
        this.loading = false;
      }
    }
  }
</script>