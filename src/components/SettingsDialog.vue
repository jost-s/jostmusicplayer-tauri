<script setup lang="ts">
defineProps<{
  selectedFolder: string | null;
  loading: boolean;
}>();
const emit = defineEmits<{
  close: [];
  "select-folder": [];
}>();
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog" role="dialog" aria-modal="true" aria-label="Settings">
      <header class="dialog-header">
        <h2>Settings</h2>
        <button class="close-btn" title="Close" @click="emit('close')">✕</button>
      </header>

      <section class="setting">
        <h3>Music Library</h3>
        <p class="hint">
          Choose the folder to scan for audio files.
        </p>
        <div class="folder-row">
          <button :disabled="loading" @click="emit('select-folder')">
            {{ loading ? "Scanning…" : selectedFolder ? "Change Folder" : "Select Folder" }}
          </button>
          <span v-if="selectedFolder" class="folder-path" :title="selectedFolder">
            {{ selectedFolder }}
          </span>
          <span v-else class="folder-path muted">No folder selected</span>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background-color: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  width: min(32rem, calc(100vw - 2rem));
  background-color: #f6f6f6;
  border-radius: 10px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.25);
  overflow: hidden;
}

.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.25rem;
  border-bottom: 1px solid #ddd;
}

.dialog-header h2 {
  font-size: 1.05rem;
  font-weight: 600;
}

.close-btn {
  width: 2rem;
  height: 2rem;
  padding: 0;
  font-size: 0.9em;
  line-height: 1;
  box-shadow: none;
}

.setting {
  padding: 1.25rem;
}

.setting h3 {
  font-size: 0.95rem;
  font-weight: 600;
  margin-bottom: 0.25rem;
}

.hint {
  font-size: 0.85em;
  color: #666;
  margin-bottom: 0.75rem;
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

.folder-path.muted {
  font-style: italic;
  color: #999;
}

@media (prefers-color-scheme: dark) {
  .dialog {
    background-color: #2f2f2f;
  }

  .dialog-header {
    border-bottom-color: #444;
  }

  .hint,
  .folder-path {
    color: #aaa;
  }

  .folder-path.muted {
    color: #888;
  }
}
</style>
