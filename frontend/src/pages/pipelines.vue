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
        { title: 'Pipeline ID', key: 'id', sortable: false },
        { title: 'Branch', key: 'git_branch', sortable: false },
        { title: 'Packages', key: 'packages', sortable: false },
        { title: 'Architectures', key: 'archs', sortable: false },
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