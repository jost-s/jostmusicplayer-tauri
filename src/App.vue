<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import TrackList, { type Track } from "./components/TrackList.vue";
import ColumnBrowser from "./components/ColumnBrowser.vue";
import { type FacetItem } from "./components/FilterPane.vue";
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

// --- Search + column-browser filters ----------------------------------------
// A search box narrows the whole library; on top of it three panes (Genre →
// Artist → Album) cascade left-to-right. Within a pane the selected values are
// OR'd; across panes (and the search) they're AND'd. An empty pane set = "All".
const searchQuery = ref("");
const selectedGenres = ref<Set<string | null>>(new Set());
const selectedArtists = ref<Set<string | null>>(new Set());
const selectedAlbums = ref<Set<string | null>>(new Set());

function matches(set: Set<string | null>, value: string | null): boolean {
  return set.size === 0 || set.has(value);
}

// Case-insensitive substring search over the visible text fields. The query is
// split on whitespace into terms that are OR'd: a track matches if any one term
// appears, so "beatles yesterday" finds both Beatles tracks and any "Yesterday".
function searchMatches(t: Track): boolean {
  const terms = searchQuery.value.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return true;
  const hay = `${t.title ?? t.filename} ${t.artist ?? ""} ${t.album ?? ""} ${t.genre ?? ""}`.toLowerCase();
  return terms.some((term) => hay.includes(term));
}

// Build a sorted, counted facet list for one field over the given rows.
// Tracks missing the field bucket under `null` ("Unknown"), which sorts last.
function buildFacet(rows: Track[], key: "genre" | "artist" | "album"): FacetItem[] {
  const counts = new Map<string | null, number>();
  for (const t of rows) {
    const value = t[key] ?? null;
    counts.set(value, (counts.get(value) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([value, count]) => ({ value, count }))
    .sort((a, b) => {
      if (a.value === null) return 1;
      if (b.value === null) return -1;
      return a.value.localeCompare(b.value);
    });
}

// Cascade scopes deliberately ignore the search box: the panes narrow each
// other, but search is a transient overlay laid on top. Keeping it out of these
// scopes is what lets a narrow search hide rows without ever discarding (pruning)
// a pane selection — clearing the search restores the full view unchanged.
const genreScoped = computed(() =>
  tracks.value.filter((t) => matches(selectedGenres.value, t.genre ?? null)),
);
const artistScoped = computed(() =>
  genreScoped.value.filter((t) => matches(selectedArtists.value, t.artist ?? null)),
);

// Pane option lists shown in the UI reflect the cascade above each pane *and* the
// search, so the visible values and counts track what's actually in the table.
const genreOptions = computed(() => buildFacet(tracks.value.filter(searchMatches), "genre"));
const artistOptions = computed(() => buildFacet(genreScoped.value.filter(searchMatches), "artist"));
const albumOptions = computed(() => buildFacet(artistScoped.value.filter(searchMatches), "album"));

// Final visible tracks: the full pane cascade plus the search filter.
const filteredTracks = computed(() =>
  artistScoped.value.filter((t) => matches(selectedAlbums.value, t.album ?? null) && searchMatches(t)),
);

// When an upstream pane (or a library rescan) removes values a downstream pane
// had selected, drop those now-invalid selections. Validity is checked against
// the search-free cascade scope, so search never triggers a prune. Pruning one
// pane recomputes the next scope, so the cascade continues down on its own.
function prune(
  selection: Set<string | null>,
  rows: Track[],
  key: "genre" | "artist" | "album",
): Set<string | null> | null {
  if (selection.size === 0) return null;
  const valid = new Set<string | null>(rows.map((r) => r[key] ?? null));
  const next = new Set([...selection].filter((v) => valid.has(v)));
  return next.size === selection.size ? null : next;
}

watch(tracks, () => {
  const pruned = prune(selectedGenres.value, tracks.value, "genre");
  if (pruned) selectedGenres.value = pruned;
});
watch(genreScoped, () => {
  const pruned = prune(selectedArtists.value, genreScoped.value, "artist");
  if (pruned) selectedArtists.value = pruned;
});
watch(artistScoped, () => {
  const pruned = prune(selectedAlbums.value, artistScoped.value, "album");
  if (pruned) selectedAlbums.value = pruned;
});

// Only the column-browser panes count as "filters" here; the search box is an
// independent control with its own native clear, so it neither shows this button
// nor is reset by it.
function clearFilters() {
  selectedGenres.value = new Set();
  selectedArtists.value = new Set();
  selectedAlbums.value = new Set();
}

// Footer summary: total tracks in the library, noting how many are shown when a
// filter or search is narrowing the view.
const trackCountLabel = computed(() => {
  const total = tracks.value.length;
  const shown = filteredTracks.value.length;
  const totalStr = total.toLocaleString();
  if (shown === total) return `${totalStr} tracks`;
  return `${shown.toLocaleString()} of ${totalStr} tracks`;
});

const hasActiveFilters = computed(
  () =>
    selectedGenres.value.size > 0 ||
    selectedArtists.value.size > 0 ||
    selectedAlbums.value.size > 0,
);

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
// Tauri event subscriptions, torn down on unmount. Without this, a hot-reload
// re-runs onMounted and stacks a second set of listeners on top of the old ones,
// so a stale `playback-ended` handler (capturing an earlier `playNext`) keeps
// firing alongside the current one.
const unlisteners: UnlistenFn[] = [];

onMounted(async () => {
  // Register listeners first: a startup scan kicked off in the backend may
  // finish before (or during) this handler, and we must not miss its events.
  unlisteners.push(
    await listen("scan-started", () => {
      scanning.value = true;
    }),
  );
  unlisteners.push(
    await listen("scan-progress", () => {
      // Surface newly-indexed tracks while the scan is still running.
      void refreshLibrary();
    }),
  );
  unlisteners.push(
    await listen("scan-finished", async () => {
      scanning.value = false;
      await refreshLibrary();
    }),
  );
  unlisteners.push(
    await listen("playback-ended", async () => {
      await playNext();
    }),
  );

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
  for (const unlisten of unlisteners) unlisten();
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
  const view = filteredTracks.value;
  const idx = current ? view.findIndex((t) => t.id === current.id) : -1;
  const next = idx >= 0 ? view[idx + 1] : undefined;
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

// Coalesce overlapping refreshes. Progress events during a scan can arrive
// faster than `get_library` returns; rather than stack queries, we run one at a
// time and remember whether another was requested, then run a single final pass
// so the table always converges to the latest DB state.
let refreshInFlight = false;
let refreshPending = false;
async function refreshLibrary() {
  if (refreshInFlight) {
    refreshPending = true;
    return;
  }
  refreshInFlight = true;
  try {
    do {
      refreshPending = false;
      tracks.value = await invoke<Track[]>("get_library", {
        sortBy: sortBy.value,
        sortDir: sortDir.value,
      });
    } while (refreshPending);
  } finally {
    refreshInFlight = false;
  }
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

async function cancelScan() {
  // The scan thread stops at the next file and emits `scan-finished`, which
  // flips `scanning` off and refreshes whatever was indexed so far.
  await invoke("cancel_scan");
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

      <button class="cog-btn" title="Settings" @click="showSettings = true">
        ⚙
      </button>
    </header>

    <div class="filter-bar">
      <input
        v-model="searchQuery"
        class="search-input"
        type="search"
        placeholder="Search library…"
      />
      <button v-if="hasActiveFilters" class="clear-filters" @click="clearFilters">
        Clear filters
      </button>

      <template v-if="currentTrack">
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
      </template>
    </div>

    <ColumnBrowser
      v-model:selected-genres="selectedGenres"
      v-model:selected-artists="selectedArtists"
      v-model:selected-albums="selectedAlbums"
      :genres="genreOptions"
      :artists="artistOptions"
      :albums="albumOptions"
    />

    <main class="library">
      <TrackList
        :tracks="filteredTracks"
        :playing-id="currentTrack?.id ?? null"
        :scanning="scanning"
        @sort-change="onSortChange"
        @play-track="playTrack"
      />
    </main>

    <footer class="status-bar">{{ trackCountLabel }}</footer>

    <SettingsDialog
      v-if="showSettings"
      :selected-folder="selectedFolder"
      :loading="scanning"
      @close="showSettings = false"
      @select-folder="selectMusicFolder"
      @cancel-scan="cancelScan"
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

.filter-bar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.5rem 1.5rem;
  border-bottom: 1px solid #ddd;
  flex-shrink: 0;
}

.search-input {
  /* Stable width on the left so the seek bar (flex: 1) fills the rest of the row;
     allowed to shrink on narrow windows but never to grow. */
  flex: 0 1 280px;
  padding: 0.4rem 0.6rem;
  font-size: 0.85em;
  font-family: inherit;
  color: inherit;
  border: 1px solid #ccc;
  border-radius: 6px;
  background-color: #fff;
  outline: none;
}

.search-input:focus {
  border-color: #396cd8;
}

/* The native WebKit clear button is a fixed dark glyph that all but disappears on
   the dark-mode field. Replace it with an SVG mask tinted by `currentColor`, so it
   stays legible in both themes. */
.search-input::-webkit-search-cancel-button {
  -webkit-appearance: none;
  appearance: none;
  height: 14px;
  width: 14px;
  background-color: currentColor;
  -webkit-mask: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'><path d='M4 4l8 8M12 4l-8 8' stroke='black' stroke-width='2' stroke-linecap='round'/></svg>")
    center / contain no-repeat;
  mask: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'><path d='M4 4l8 8M12 4l-8 8' stroke='black' stroke-width='2' stroke-linecap='round'/></svg>")
    center / contain no-repeat;
  opacity: 0.45;
  transition: opacity 0.15s;
}

.search-input::-webkit-search-cancel-button:hover {
  opacity: 0.8;
}

.clear-filters {
  font-size: 0.75em;
  padding: 0.25em 0.7em;
}

.library {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0 0.5rem;
}

.status-bar {
  flex-shrink: 0;
  padding: 0.35rem 1.5rem;
  border-top: 1px solid #ddd;
  font-size: 0.75em;
  color: #666;
  text-align: center;
  font-variant-numeric: tabular-nums;
  user-select: none;
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
  .filter-bar {
    border-bottom-color: #444;
  }

  .status-bar {
    border-top-color: #444;
    color: #aaa;
  }

  .search-input {
    background-color: #1f1f1f;
    border-color: #555;
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
