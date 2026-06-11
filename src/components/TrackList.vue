<script setup lang="ts">
import { ref } from "vue";

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

const props = defineProps<{ tracks: Track[]; playingId: number | null }>();
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
</script>

<template>
  <div class="track-list">
    <table>
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
      <tbody>
        <tr v-if="props.tracks.length === 0">
          <td colspan="6" class="empty">No tracks found.</td>
        </tr>
        <tr
          v-for="track in props.tracks"
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
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.track-list {
  width: 100%;
  height: 100%;
  overflow: auto;
}

table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.9em;
}

thead th {
  position: sticky;
  top: 0;
  background-color: #f6f6f6;
  z-index: 1;
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
}

tbody tr:hover {
  background-color: rgba(0, 0, 0, 0.04);
}

tbody tr.playing {
  background-color: rgba(57, 108, 216, 0.12);
}

tbody tr.playing td.title {
  color: #396cd8;
}

tbody td {
  padding: 0.4rem 0.75rem;
  border-bottom: 1px solid #eee;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 240px;
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
