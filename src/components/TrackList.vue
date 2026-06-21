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
}

const props = defineProps<{
  tracks: Track[];
  playingId: number | null;
  scanning?: boolean;
}>();
const emit = defineEmits<{
  "sort-change": [sortBy: string, sortDir: "asc" | "desc"];
  "play-track": [track: Track];
}>();

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

watch(
  () => props.tracks,
  async () => {
    if (!scrollToPlayingPending) return;
    scrollToPlayingPending = false;
    await nextTick();
    scrollPlayingIntoView();
  },
);

let resizeObserver: ResizeObserver | undefined;
onMounted(() => {
  if (!scroller.value) return;
  viewportHeight.value = scroller.value.clientHeight;
  resizeObserver = new ResizeObserver(() => {
    if (scroller.value) viewportHeight.value = scroller.value.clientHeight;
  });
  resizeObserver.observe(scroller.value);
});
onBeforeUnmount(() => resizeObserver?.disconnect());
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
}
</style>
