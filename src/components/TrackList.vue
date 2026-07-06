<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from "vue";

export interface Track {
  id: number;
  path: string;
  filename: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  year: number | null;
  track_num: number | null;
  duration: number | null;
  genre: string | null;
}

const props = defineProps<{
  tracks: Track[];
  playingId: number | null;
  scanning?: boolean;
}>();
const emit = defineEmits<{
  "sort-change": [sortBy: string, sortDir: "asc" | "desc"];
  "play-track": [track: Track];
  "edit-track": [track: Track];
}>();

// Right-click context menu. `null` when closed; otherwise holds the cursor
// position and the track it was opened on.
const contextMenu = ref<{ x: number; y: number; track: Track } | null>(null);

function openContextMenu(event: MouseEvent, track: Track) {
  contextMenu.value = { x: event.clientX, y: event.clientY, track };
}
function closeContextMenu() {
  contextMenu.value = null;
}
function editFromMenu() {
  if (contextMenu.value) emit("edit-track", contextMenu.value.track);
  closeContextMenu();
}

const sortBy = ref("artist");
const sortDir = ref<"asc" | "desc">("asc");

function toggleSort(col: string) {
  if (sortBy.value === col) {
    sortDir.value = sortDir.value === "asc" ? "desc" : "asc";
  } else {
    sortBy.value = col;
    sortDir.value = "asc";
  }
  // The re-sorted list arrives asynchronously via the `tracks` prop; remember
  // that the user just sorted so we can scroll to the playing track once it does.
  scrollToPlayingPending = true;
  emit("sort-change", sortBy.value, sortDir.value);
}

function formatDuration(seconds: number | null): string {
  if (seconds == null) return "—";
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

const COLUMNS: { key: string; label: string }[] = [
  { key: "track_num", label: "#" },
  { key: "title", label: "Title" },
  { key: "artist", label: "Artist" },
  { key: "album", label: "Album" },
  { key: "year", label: "Year" },
  { key: "duration", label: "Duration" },
];

// Virtualized rendering: only the rows in (and just around) the viewport exist in
// the DOM. Without this, large libraries put tens of thousands of <tr>s on the
// page, and any style change — notably toggling the `playing` highlight — forces
// the engine to recalc/reflow the whole table, which can stall for seconds.
// Fixed-height rows let us map scroll position to a slice with simple arithmetic.
const ROW_HEIGHT = 32; // px; must match `tbody tr` height in the stylesheet
const OVERSCAN = 8; // extra rows above/below the viewport to avoid blank edges

const scroller = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportHeight = ref(0);

const startIndex = computed(() =>
  Math.max(0, Math.floor(scrollTop.value / ROW_HEIGHT) - OVERSCAN),
);
const endIndex = computed(() =>
  Math.min(
    props.tracks.length,
    Math.ceil((scrollTop.value + viewportHeight.value) / ROW_HEIGHT) + OVERSCAN,
  ),
);
const visibleTracks = computed(() => props.tracks.slice(startIndex.value, endIndex.value));
// Spacer heights stand in for the rows we don't render, keeping the scrollbar and
// total height correct.
const topPad = computed(() => startIndex.value * ROW_HEIGHT);
const bottomPad = computed(() => (props.tracks.length - endIndex.value) * ROW_HEIGHT);

function onScroll() {
  if (scroller.value) scrollTop.value = scroller.value.scrollTop;
  // Scrolling would leave the menu floating over the wrong row; dismiss it.
  closeContextMenu();
}

// Set when the user toggles a column header; consumed once the re-sorted `tracks`
// prop lands so we scroll the now-playing track into view at its new position.
let scrollToPlayingPending = false;

function scrollPlayingIntoView() {
  if (props.playingId == null || !scroller.value) return;
  const index = props.tracks.findIndex((t) => t.id === props.playingId);
  if (index < 0) return;
  // Center the row in the viewport when possible, clamped to valid scroll range.
  const maxTop = Math.max(0, props.tracks.length * ROW_HEIGHT - viewportHeight.value);
  const target = index * ROW_HEIGHT - (viewportHeight.value - ROW_HEIGHT) / 2;
  scroller.value.scrollTop = Math.min(maxTop, Math.max(0, target));
}

// Bring the playing track into view only when it isn't already fully visible, so
// skipping (next/previous, including via media keys) reveals an off-screen track
// without yanking the view when the user double-clicks a row that's already shown.
function ensurePlayingVisible() {
  if (props.playingId == null || !scroller.value) return;
  const index = props.tracks.findIndex((t) => t.id === props.playingId);
  if (index < 0) return;
  const rowTop = index * ROW_HEIGHT;
  const viewTop = scroller.value.scrollTop;
  if (rowTop >= viewTop && rowTop + ROW_HEIGHT <= viewTop + viewportHeight.value) return;
  scrollPlayingIntoView();
}

// Follow the playing track as it changes (skip, auto-advance) by scrolling it into
// view. The `tracks`-watch above already handles the re-sort case via its pending
// flag, so this only needs to react to the id itself moving.
watch(
  () => props.playingId,
  async () => {
    await nextTick();
    ensurePlayingVisible();
  },
);

watch(
  () => props.tracks,
  async () => {
    await nextTick();
    if (scrollToPlayingPending) {
      scrollToPlayingPending = false;
      scrollPlayingIntoView();
    } else if (scroller.value) {
      // Preserve the user's scroll position across refreshes — notably the
      // progressive updates emitted while a scan runs, which would otherwise keep
      // snapping back to the top. Only clamp when the new (possibly shorter) list
      // no longer reaches the current offset, so the viewport can't be stranded
      // past the end.
      const maxTop = Math.max(0, props.tracks.length * ROW_HEIGHT - viewportHeight.value);
      if (scroller.value.scrollTop > maxTop) {
        scroller.value.scrollTop = maxTop;
        scrollTop.value = maxTop;
      }
    }
  },
);

// Let the parent request that the playing track be scrolled into view once the
// next `tracks` update lands — e.g. after clearing filters re-expands the list.
// Reuses the same pending-flag path as a column re-sort.
defineExpose({
  queueScrollToPlaying() {
    scrollToPlayingPending = true;
  },
});

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") closeContextMenu();
}

let resizeObserver: ResizeObserver | undefined;
onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  if (!scroller.value) return;
  viewportHeight.value = scroller.value.clientHeight;
  resizeObserver = new ResizeObserver(() => {
    if (scroller.value) viewportHeight.value = scroller.value.clientHeight;
  });
  resizeObserver.observe(scroller.value);
});
onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div class="track-list">
    <!-- Header lives in its own table outside the scroll container, so a hovered
         row scrolled to the top can't repaint over it (a WebKit sticky-header
         bug). Both tables use identical fixed column widths to stay aligned. -->
    <table class="header-table">
      <colgroup>
        <col
          v-for="col in COLUMNS"
          :key="col.key"
          :class="`col-${col.key}`"
        />
      </colgroup>
      <thead>
        <tr>
          <th
            v-for="col in COLUMNS"
            :key="col.key"
            :class="{ active: sortBy === col.key }"
            @click="toggleSort(col.key)"
          >
            {{ col.label }}
            <span v-if="sortBy === col.key" class="sort-arrow">
              {{ sortDir === "asc" ? "↑" : "↓" }}
            </span>
          </th>
        </tr>
      </thead>
    </table>
    <div ref="scroller" class="body-scroll" @scroll="onScroll">
      <table class="body-table">
        <colgroup>
          <col
            v-for="col in COLUMNS"
            :key="col.key"
            :class="`col-${col.key}`"
          />
        </colgroup>
        <tbody>
          <tr v-if="props.tracks.length === 0">
            <td colspan="6" class="empty">
              {{ props.scanning ? "Scanning…" : "No tracks found." }}
            </td>
          </tr>
          <tr v-if="topPad > 0" class="spacer" :style="{ height: topPad + 'px' }">
            <td colspan="6"></td>
          </tr>
          <tr
            v-for="track in visibleTracks"
            :key="track.id"
            :class="{ playing: track.id === props.playingId }"
            @dblclick="emit('play-track', track)"
            @contextmenu.prevent="openContextMenu($event, track)"
          >
            <td class="num">{{ track.track_num ?? "—" }}</td>
            <td class="title">{{ track.title ?? track.filename }}</td>
            <td>{{ track.artist ?? "—" }}</td>
            <td>{{ track.album ?? "—" }}</td>
            <td class="num">{{ track.year ?? "—" }}</td>
            <td class="num">{{ formatDuration(track.duration) }}</td>
          </tr>
          <tr v-if="bottomPad > 0" class="spacer" :style="{ height: bottomPad + 'px' }">
            <td colspan="6"></td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Right-click context menu. The full-screen backdrop swallows the next
         click (or right-click) anywhere to dismiss the menu. -->
    <template v-if="contextMenu">
      <div
        class="context-backdrop"
        @click="closeContextMenu"
        @contextmenu.prevent="closeContextMenu"
      ></div>
      <ul
        class="context-menu"
        :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }"
      >
        <li @click="editFromMenu">Edit tags…</li>
      </ul>
    </template>
  </div>
</template>

<style scoped>
.track-list {
  width: 100%;
  height: 100%;
  /* Column layout: fixed header on top, scrolling body fills the rest. */
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.body-scroll {
  flex: 1;
  overflow: auto;
}

table {
  width: 100%;
  border-collapse: separate;
  border-spacing: 0;
  font-size: 0.9em;
  /* Fixed layout keeps column widths stable as the virtualized row slice changes
     (with `auto` they'd jump because only a subset of rows is measured) and keeps
     the separate header and body tables aligned to identical column widths. */
  table-layout: fixed;
}

.col-track_num {
  width: 56px;
}
.col-title {
  width: 34%;
}
.col-artist,
.col-album {
  width: 26%;
}
.col-year {
  width: 64px;
}
.col-duration {
  width: 88px;
}

thead th {
  background-color: #f6f6f6;
  text-align: left;
  padding: 0.5rem 0.75rem;
  border-bottom: 2px solid #ccc;
  cursor: default;
  user-select: none;
  white-space: nowrap;
}

thead th:hover {
  background-color: rgba(0, 0, 0, 0.05);
}

thead th.active {
  color: #396cd8;
}

.sort-arrow {
  margin-left: 0.25rem;
  font-size: 0.8em;
}

tbody tr {
  cursor: default;
  user-select: none;
  /* Must equal ROW_HEIGHT in the script for the virtual-scroll math to line up. */
  height: 32px;
}

tbody tr:hover {
  background-color: rgba(0, 0, 0, 0.04);
}

/* Empty rows that pad the scroll height for the off-screen (unrendered) tracks. */
tbody tr.spacer,
tbody tr.spacer:hover {
  background: none;
}

tbody tr.spacer td {
  padding: 0;
  border: 0;
}

tbody tr.playing {
  background-color: rgba(57, 108, 216, 0.12);
}

tbody tr.playing td.title {
  color: #396cd8;
}

tbody td {
  /* No vertical padding: the fixed 32px row height (above) controls row size, and
     extra padding would push content past it and break the virtual-scroll math. */
  padding: 0 0.75rem;
  border-bottom: 1px solid #eee;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

td.num {
  text-align: right;
  max-width: 80px;
  color: #666;
}

td.title {
  font-weight: 500;
}

td.empty {
  text-align: center;
  padding: 2rem;
  color: #999;
}

/* Transparent layer over the whole window so any click dismisses the menu. */
.context-backdrop {
  position: fixed;
  inset: 0;
  z-index: 200;
}

.context-menu {
  position: fixed;
  z-index: 201;
  min-width: 9rem;
  list-style: none;
  padding: 0.25rem;
  background-color: #fff;
  border: 1px solid #ddd;
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.2);
  font-size: 0.9em;
  user-select: none;
}

.context-menu li {
  padding: 0.4rem 0.75rem;
  border-radius: 4px;
  cursor: default;
  white-space: nowrap;
}

.context-menu li:hover {
  background-color: #396cd8;
  color: #fff;
}

@media (prefers-color-scheme: dark) {
  thead th {
    background-color: #2f2f2f;
    border-bottom-color: #555;
  }

  thead th:hover {
    background-color: rgba(255, 255, 255, 0.07);
  }

  tbody tr:hover {
    background-color: rgba(255, 255, 255, 0.05);
  }

  tbody tr.playing {
    background-color: rgba(122, 162, 247, 0.2);
  }

  tbody tr.playing td.title {
    color: #7aa2f7;
  }

  tbody td {
    border-bottom-color: #3a3a3a;
  }

  td.num {
    color: #999;
  }

  .context-menu {
    background-color: #2f2f2f;
    border-color: #444;
  }

  .context-menu li:hover {
    background-color: #7aa2f7;
    color: #1f1f1f;
  }
}
</style>
