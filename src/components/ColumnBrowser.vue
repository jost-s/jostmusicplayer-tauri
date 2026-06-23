<script setup lang="ts">
import FilterPane, { type FacetItem } from "./FilterPane.vue";

defineProps<{
  genres: FacetItem[];
  artists: FacetItem[];
  albums: FacetItem[];
  selectedGenres: Set<string | null>;
  selectedArtists: Set<string | null>;
  selectedAlbums: Set<string | null>;
}>();

// Per-pane v-model: App owns the selection state and the cascade logic.
defineEmits<{
  "update:selectedGenres": [next: Set<string | null>];
  "update:selectedArtists": [next: Set<string | null>];
  "update:selectedAlbums": [next: Set<string | null>];
}>();
</script>

<template>
  <div class="column-browser">
    <FilterPane
      title="Genre"
      :items="genres"
      :selected="selectedGenres"
      @update:selected="$emit('update:selectedGenres', $event)"
    />
    <FilterPane
      title="Artist"
      :items="artists"
      :selected="selectedArtists"
      @update:selected="$emit('update:selectedArtists', $event)"
    />
    <FilterPane
      title="Album"
      :items="albums"
      :selected="selectedAlbums"
      @update:selected="$emit('update:selectedAlbums', $event)"
    />
  </div>
</template>

<style scoped>
.column-browser {
  display: flex;
  height: 200px;
  flex-shrink: 0;
  border-bottom: 1px solid #ddd;
}

@media (prefers-color-scheme: dark) {
  .column-browser {
    border-bottom-color: #444;
  }
}
</style>
