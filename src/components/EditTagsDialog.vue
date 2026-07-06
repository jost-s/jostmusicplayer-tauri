<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Track } from "./TrackList.vue";

const props = defineProps<{
  track: Track;
}>();
const emit = defineEmits<{
  close: [];
  saved: [];
}>();

// Local editable copies, seeded from the track. Text fields use "" for null so
// the inputs bind cleanly; numbers use "" when absent.
const title = ref(props.track.title ?? "");
const artist = ref(props.track.artist ?? "");
const album = ref(props.track.album ?? "");
const genre = ref(props.track.genre ?? "");
const year = ref(props.track.year != null ? String(props.track.year) : "");
const trackNum = ref(props.track.track_num != null ? String(props.track.track_num) : "");

const saving = ref(false);
const error = ref<string | null>(null);

// A number-typed <input> can hand `v-model` back an actual number (not a
// string), and an empty/invalid field yields "" — so these helpers accept any
// value and normalize defensively rather than assuming a string.

// Blank -> null (clears the tag); otherwise the trimmed string.
function textField(value: unknown): string | null {
  const trimmed = value == null ? "" : String(value).trim();
  return trimmed === "" ? null : trimmed;
}

// Non-negative integers up to `max`; anything else (blank, decimal, negative,
// out-of-range, or non-numeric) becomes null so we never send a value the
// backend would reject. `max` keeps year within i32 and track # within u32.
function numField(value: unknown, max: number): number | null {
  if (value == null || String(value).trim() === "") return null;
  const n = Number(value);
  return Number.isInteger(n) && n >= 0 && n <= max ? n : null;
}

const I32_MAX = 2147483647;
const U32_MAX = 4294967295;

async function save() {
  saving.value = true;
  error.value = null;
  try {
    await invoke("update_track_tags", {
      path: props.track.path,
      title: textField(title.value),
      artist: textField(artist.value),
      album: textField(album.value),
      year: numField(year.value, I32_MAX),
      trackNum: numField(trackNum.value, U32_MAX),
      genre: textField(genre.value),
    });
    emit("saved");
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog" role="dialog" aria-modal="true" aria-label="Edit tags">
      <header class="dialog-header">
        <h2>Edit Tags</h2>
        <button class="close-btn" title="Close" @click="emit('close')">✕</button>
      </header>

      <!-- novalidate: every tag field is optional, so the browser must never
           block submit on native constraints (e.g. an empty number input). -->
      <form class="fields" novalidate @submit.prevent="save">
        <p class="filename" :title="track.path">{{ track.filename }}</p>

        <label class="field">
          <span>Title</span>
          <input v-model="title" type="text" autofocus />
        </label>
        <label class="field">
          <span>Artist</span>
          <input v-model="artist" type="text" />
        </label>
        <label class="field">
          <span>Album</span>
          <input v-model="album" type="text" />
        </label>
        <div class="field-row">
          <label class="field">
            <span>Year</span>
            <input v-model="year" type="number" min="0" inputmode="numeric" />
          </label>
          <label class="field">
            <span>Track #</span>
            <input v-model="trackNum" type="number" min="0" inputmode="numeric" />
          </label>
        </div>
        <label class="field">
          <span>Genre</span>
          <input v-model="genre" type="text" />
        </label>

        <p v-if="error" class="error">{{ error }}</p>

        <footer class="actions">
          <button type="button" class="secondary" :disabled="saving" @click="emit('close')">
            Cancel
          </button>
          <button type="submit" class="primary" :disabled="saving">
            {{ saving ? "Saving…" : "Save" }}
          </button>
        </footer>
      </form>
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
  width: min(28rem, calc(100vw - 2rem));
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

.fields {
  padding: 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.filename {
  font-size: 0.8em;
  color: #666;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  margin-bottom: 0.25rem;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  flex: 1;
}

.field > span {
  font-size: 0.8em;
  color: #666;
  user-select: none;
}

.field input {
  padding: 0.4rem 0.6rem;
  font-size: 0.9em;
  border: 1px solid #ccc;
  border-radius: 6px;
  background-color: #fff;
  color: inherit;
  cursor: default;
}

.field input:focus {
  outline: none;
  border-color: #396cd8;
}

.field-row {
  display: flex;
  gap: 0.75rem;
}

.error {
  font-size: 0.8em;
  color: #b00020;
  white-space: pre-wrap;
  word-break: break-word;
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 0.5rem;
}

.primary {
  border-color: #396cd8;
  background-color: #396cd8;
  color: #fff;
}

.primary:hover:not(:disabled) {
  background-color: #2f5bc0;
}

@media (prefers-color-scheme: dark) {
  .dialog {
    background-color: #2f2f2f;
  }

  .dialog-header {
    border-bottom-color: #444;
  }

  .filename,
  .field > span {
    color: #aaa;
  }

  .field input {
    background-color: #1f1f1f;
    border-color: #444;
  }

  .field input:focus {
    border-color: #7aa2f7;
  }

  .primary {
    border-color: #7aa2f7;
    background-color: #3a5bbf;
  }

  .primary:hover:not(:disabled) {
    background-color: #4a6fd0;
  }
}
</style>
