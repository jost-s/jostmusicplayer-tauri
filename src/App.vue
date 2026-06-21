<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import TrackList, { type Track } from "./components/TrackList.vue";
import SettingsDialog from "./components/SettingsDialog.vue";

const selectedFolder = ref<string | null>(null);
const tracks = ref<Track[]>([]);
const scanning = ref(false);
const showSettings = ref(false);
const sortBy = ref("artist");
const sortDir = ref<"asc" | "desc">("asc");
const currentTrack = ref<Track | null>(null);
const isPlaying = ref(false);
const position = ref(0);
const duration = ref(0);
const showRemaining = ref(false);

const progressPercent = computed(() =>
  duration.value > 0 ? Math.min(100, (position.value / duration.value) * 100) : 0,
);

const elapsedLabel = computed(() => {
  if (showRemaining.value && duration.value > 0) {
    return "-" + formatTime(Math.max(0, duration.value - position.value));
  }
  return formatTime(position.value);
});

let posTimer: ReturnType<typeof setInterval> | undefined;

onMounted(async () => {
  // Register listeners first: a startup scan kicked off in the backend may
  // finish before (or during) this handler, and we must not miss its events.
  await listen("scan-started", () => {
    scanning.value = true;
  });
  await listen("scan-finished", async () => {
    scanning.value = false;
    await refreshLibrary();
  });
  await listen("playback-ended", async () => {
    await playNext();
  });

  selectedFolder.value = await invoke<string | null>("get_library_folder");
  if (selectedFolder.value) {
    // Show the previous session's library immediately; the background scan
    // (already running) will refresh it via `scan-finished`.
    scanning.value = await invoke<boolean>("is_scanning");
    await refreshLibrary();
  } else {
    // First launch with no library configured — prompt the user to pick one.
    showSettings.value = true;
  }

  posTimer = setInterval(async () => {
    if (currentTrack.value && isPlaying.value) {
      position.value = await invoke<number>("playback_position");
    }
  }, 500);
});

onUnmounted(() => {
  if (posTimer !== undefined) clearInterval(posTimer);
});

async function playTrack(track: Track) {
  const total = await invoke<number | null>("play_track", { path: track.path });
  currentTrack.value = track;
  isPlaying.value = true;
  position.value = 0;
  // Prefer the duration the decoder reports; fall back to the scanned tag length.
  duration.value = total ?? track.duration ?? 0;
}

// Called when a track finishes on its own: continue with whatever is currently
// shown in the table (respecting the active sort/filter), advancing to the row
// after the one that just played. Stops if there's nothing after it.
async function playNext() {
  const current = currentTrack.value;
  const idx = current ? tracks.value.findIndex((t) => t.id === current.id) : -1;
  const next = idx >= 0 ? tracks.value[idx + 1] : undefined;
  if (next) {
    await playTrack(next);
  } else {
    isPlaying.value = false;
    position.value = 0;
    currentTrack.value = null;
    duration.value = 0;
  }
}

async function togglePlayback() {
  isPlaying.value = await invoke<boolean>("toggle_playback");
}

async function seekTo(e: MouseEvent) {
  if (duration.value <= 0) return;
  const bar = e.currentTarget as HTMLElement;
  const rect = bar.getBoundingClientRect();
  const fraction = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
  const seconds = fraction * duration.value;
  position.value = seconds;
  await invoke("seek", { seconds });
}

function formatTime(seconds: number): string {
  const total = Math.floor(seconds);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

async function refreshLibrary() {
  tracks.value = await invoke<Track[]>("get_library", {
    sortBy: sortBy.value,
    sortDir: sortDir.value,
  });
}

async function selectMusicFolder() {
  const folder = await open({
    directory: true,
    multiple: false,
    title: "Select Music Folder",
  });
  if (typeof folder !== "string") return;

  selectedFolder.value = folder;
  await invoke("set_library_folder", { folder });
  // Kick off a background scan; `scan-started`/`scan-finished` drive the
  // indicator and refresh the table when it completes.
  scanning.value = true;
  await invoke("scan_library");
}

async function onSortChange(by: string, dir: "asc" | "desc") {
  sortBy.value = by;
  sortDir.value = dir;
  await refreshLibrary();
}
</script>

<template>
  <div class="app">
    <header class="toolbar">
      <h1>Jost Music Player</h1>
      <span v-if="scanning" class="scanning-badge" title="Scanning library…">
        <span class="scanning-dot"></span>
        Scanning…
      </span>
      <div class="transport">
        <button
          class="play-toggle"
          :disabled="!currentTrack"
          :title="isPlaying ? 'Pause' : 'Play'"
          @click="togglePlayback"
        >
          {{ isPlaying ? "⏸" : "▶" }}
        </button>
        <div v-if="currentTrack" class="now-playing">
          <span class="np-title">{{ currentTrack.title ?? currentTrack.filename }}</span>
          <span v-if="currentTrack.artist" class="np-artist">{{ currentTrack.artist }}</span>
        </div>
      </div>

      <button
        class="cog-btn"
        :class="{ spin: scanning }"
        title="Settings"
        @click="showSettings = true"
      >
        ⚙
      </button>
    </header>

    <div v-if="currentTrack" class="progress-row">
      <span
        class="time clickable"
        :title="showRemaining ? 'Show elapsed time' : 'Show remaining time'"
        @click="showRemaining = !showRemaining"
        >{{ elapsedLabel }}</span
      >
      <div
        class="progress-bar"
        :class="{ disabled: duration <= 0 }"
        @click="seekTo"
      >
        <div class="progress-fill" :style="{ width: progressPercent + '%' }"></div>
        <div class="progress-knob" :style="{ left: progressPercent + '%' }"></div>
      </div>
      <span class="time">{{ duration > 0 ? formatTime(duration) : "—" }}</span>
    </div>

    <main class="library">
      <TrackList
        :tracks="tracks"
        :playing-id="currentTrack?.id ?? null"
        :scanning="scanning"
        @sort-change="onSortChange"
        @play-track="playTrack"
      />
    </main>

    <SettingsDialog
      v-if="showSettings"
      :selected-folder="selectedFolder"
      :loading="scanning"
      @close="showSettings = false"
      @select-folder="selectMusicFolder"
    />
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

.scanning-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.75em;
  color: #666;
  white-space: nowrap;
}

.scanning-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background-color: #396cd8;
  animation: pulse 1.2s ease-in-out infinite;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 0.3;
    transform: scale(0.8);
  }
  50% {
    opacity: 1;
    transform: scale(1.1);
  }
}

.cog-btn {
  width: 2.2rem;
  height: 2.2rem;
  padding: 0;
  font-size: 1.1em;
  line-height: 1;
  margin-left: auto;
}

.cog-btn.spin {
  animation: spin 1.5s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.transport {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  overflow: hidden;
}

.play-toggle {
  width: 2.2rem;
  padding: 0.4em 0;
  font-size: 0.9em;
  text-align: center;
}

.now-playing {
  display: flex;
  flex-direction: column;
  line-height: 1.2;
  overflow: hidden;
}

.np-title {
  font-size: 0.85em;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.np-artist {
  font-size: 0.75em;
  color: #666;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.progress-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.5rem 1.5rem;
  border-bottom: 1px solid #ddd;
  flex-shrink: 0;
}

.time {
  font-size: 0.75em;
  color: #666;
  font-variant-numeric: tabular-nums;
  min-width: 4ch;
  text-align: center;
}

.time.clickable {
  cursor: default;
  user-select: none;
  transition: color 0.15s;
}

.time.clickable:hover {
  color: #396cd8;
}

.progress-bar {
  position: relative;
  flex: 1;
  height: 6px;
  border-radius: 3px;
  background-color: #ddd;
  cursor: default;
}

.progress-bar.disabled {
  cursor: default;
}

.progress-fill {
  height: 100%;
  background-color: #396cd8;
  border-radius: 3px;
}

.progress-knob {
  position: absolute;
  top: 50%;
  width: 12px;
  height: 12px;
  margin-left: -6px;
  border-radius: 50%;
  background-color: #396cd8;
  transform: translateY(-50%);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  pointer-events: none;
  transition: transform 0.1s ease;
}

.progress-bar:hover .progress-knob {
  transform: translateY(-50%) scale(1.3);
}

.progress-bar.disabled .progress-knob {
  display: none;
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

  .toolbar,
  .progress-row {
    border-bottom-color: #444;
  }

  .np-artist,
  .time,
  .scanning-badge {
    color: #aaa;
  }

  .progress-bar {
    background-color: #4a4a4a;
  }

  .time.clickable:hover {
    color: #7aa2f7;
  }

  button {
    color: #ffffff;
    background-color: #0f0f0f98;
  }
}
</style>
