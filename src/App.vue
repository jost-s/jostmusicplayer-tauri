<script setup lang="ts">
import { ref, onMounted } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import TrackList, { type Track } from "./components/TrackList.vue";

const selectedFolder = ref<string | null>(null);
const tracks = ref<Track[]>([]);
const loading = ref(false);

onMounted(async () => {
  selectedFolder.value = await invoke<string | null>("get_library_folder");
  if (selectedFolder.value) {
    tracks.value = await invoke<Track[]>("get_library", {
      sortBy: "artist",
      sortDir: "asc",
    });
  }
});

async function selectMusicFolder() {
  const folder = await open({
    directory: true,
    multiple: false,
    title: "Select Music Folder",
  });
  if (typeof folder !== "string") return;

  selectedFolder.value = folder;
  loading.value = true;
  try {
    await invoke("set_library_folder", { folder });
    await invoke("scan_library");
    tracks.value = await invoke<Track[]>("get_library", {
      sortBy: "artist",
      sortDir: "asc",
    });
  } finally {
    loading.value = false;
  }
}

async function onSortChange(sortBy: string, sortDir: "asc" | "desc") {
  tracks.value = await invoke<Track[]>("get_library", { sortBy, sortDir });
}
</script>

<template>
  <div class="app">
    <header class="toolbar">
      <h1>Jost Music Player</h1>
      <div class="folder-row">
        <button :disabled="loading" @click="selectMusicFolder">
          {{ loading ? "Scanning…" : "Select Music Folder" }}
        </button>
        <span v-if="selectedFolder" class="folder-path">{{ selectedFolder }}</span>
      </div>
    </header>

    <main class="library">
      <TrackList :tracks="tracks" @sort-change="onSortChange" />
    </main>
  </div>
</template>

<style>
:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 24px;
  font-weight: 400;
  color: #0f0f0f;
  background-color: #f6f6f6;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  -webkit-text-size-adjust: 100%;
}

* {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

.app {
  display: flex;
  flex-direction: column;
  height: 100vh;
}

.toolbar {
  padding: 1rem 1.5rem;
  border-bottom: 1px solid #ddd;
  display: flex;
  align-items: center;
  gap: 1.5rem;
  flex-shrink: 0;
}

.toolbar h1 {
  font-size: 1.1rem;
  font-weight: 600;
  white-space: nowrap;
}

.folder-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  overflow: hidden;
}

.folder-path {
  font-size: 0.8em;
  color: #666;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.library {
  flex: 1;
  overflow-y: auto;
  padding: 0 0.5rem;
}

button {
  border-radius: 6px;
  border: 1px solid transparent;
  padding: 0.4em 1em;
  font-size: 0.9em;
  font-weight: 500;
  font-family: inherit;
  color: #0f0f0f;
  background-color: #ffffff;
  cursor: default;
  transition: border-color 0.2s;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  outline: none;
  white-space: nowrap;
  flex-shrink: 0;
}

button:hover:not(:disabled) {
  border-color: #396cd8;
}

button:disabled {
  opacity: 0.5;
  cursor: default;
}

@media (prefers-color-scheme: dark) {
  :root {
    color: #f6f6f6;
    background-color: #2f2f2f;
  }

  .toolbar {
    border-bottom-color: #444;
  }

  .folder-path {
    color: #aaa;
  }

  button {
    color: #ffffff;
    background-color: #0f0f0f98;
  }
}
</style>
