<script setup lang="ts">
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";

const selectedFolder = ref<string | null>(null);

async function selectMusicFolder() {
  const folder = await open({
    directory: true,
    multiple: false,
    title: "Select Music Folder",
  });

  if (typeof folder === "string") {
    selectedFolder.value = folder;
  }
}
</script>

<template>
  <main class="container">
    <h1>Jost Music Player</h1>

    <div class="library-setup">
      <p>Select the folder where your music is stored to build your library.</p>
      <button @click="selectMusicFolder">Select Music Folder</button>
      <p v-if="selectedFolder" class="folder-path">{{ selectedFolder }}</p>
    </div>
  </main>
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

.container {
  margin: 0;
  padding-top: 10vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
}

h1 {
  margin-bottom: 2rem;
}

.library-setup {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
}

button {
  border-radius: 8px;
  border: 1px solid transparent;
  padding: 0.6em 1.4em;
  font-size: 1em;
  font-weight: 500;
  font-family: inherit;
  color: #0f0f0f;
  background-color: #ffffff;
  cursor: pointer;
  transition: border-color 0.25s;
  box-shadow: 0 2px 2px rgba(0, 0, 0, 0.2);
  outline: none;
}

button:hover {
  border-color: #396cd8;
}

button:active {
  border-color: #396cd8;
  background-color: #e8e8e8;
}

.folder-path {
  font-size: 0.9em;
  color: #555;
  word-break: break-all;
  max-width: 500px;
}

@media (prefers-color-scheme: dark) {
  :root {
    color: #f6f6f6;
    background-color: #2f2f2f;
  }

  button {
    color: #ffffff;
    background-color: #0f0f0f98;
  }

  button:active {
    background-color: #0f0f0f69;
  }

  .folder-path {
    color: #aaa;
  }
}
</style>
