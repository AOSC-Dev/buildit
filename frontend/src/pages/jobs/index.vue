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
          :sort-by="sortBy"
          item-value="id"
          @update:options="loadItems">
          <template #item.id="{ item }">
            <router-link :to="{ path: `/jobs/${(item as Job).id}` }">
              {{ (item as Job).id }}
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
  import { hostname } from '@/common';

  interface SortItem {
    key: string;
    order: 'asc' | 'desc';
  }

  interface LoadItemsOpts {
    page: number;
    itemsPerPage: number;
    sortBy: SortItem[];
  }

  interface Job {
    id: number;
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
      serverItems: [],
      sortBy: [{
        key: 'id',
        order: 'desc'
      } as SortItem]
    }),
    methods: {
      async loadItems (opts: LoadItemsOpts) {
        this.loading = true;
        let url = hostname + `/api/job/list?page=${opts.page}&items_per_page=${opts.itemsPerPage}`;
        if (opts.sortBy.length > 0) {
          url += `&sort_key=${opts.sortBy[0].key}&sort_order=${opts.sortBy[0].order}`;
        }
        let data = (await axios.get(url)).data;
        this.totalItems = data.total_items;
        this.serverItems = data.items;
        this.loading = false;
      }
    }
  }
</script>