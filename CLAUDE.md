# jostmusicplayer

A cross-platform desktop music library manager and player built with Tauri 2, Vue 3, and Rust.

## Project Goals

- Music library management: scan folders, index tracks
- Playback
- ID3v3 tag editing
- Possibly a plugin system (TBD)

## Architecture

```
jostmusicplayer/
├── src/                  # Vue 3 frontend (TypeScript)
│   ├── App.vue
│   └── main.ts
├── src-tauri/            # Rust backend
│   ├── src/
│   │   ├── lib.rs        # Tauri app setup, command registration
│   │   └── main.rs
│   ├── Cargo.toml
│   └── tauri.conf.json
├── package.json
└── vite.config.ts
```

**Frontend:** Vue 3 with `<script setup>` (Composition API), TypeScript, Vite  
**Backend:** Rust, Tauri 2  
**IPC:** Tauri commands (`invoke`) and events — frontend calls Rust via `@tauri-apps/api`

## Dev Commands

```bash
# Run in dev mode (starts Vite + Tauri watcher)
npm run tauri dev

# Type-check frontend
npx vue-tsc --noEmit

# Build for production
npm run tauri build

# Rust-only check (faster than full build)
cd src-tauri && cargo check
```

## Releasing

Releases are driven by [release-please](https://github.com/googleapis/release-please)
from the conventional-commit messages on `main` — **never bump versions by hand**.

Every push to `main` updates one standing `chore(main): release X.Y.Z` PR holding
the version bumps (`package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`,
`src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`) and the new `CHANGELOG.md` section.
Merging that PR cuts a draft GitHub release, builds the bundles onto it, and then
publishes it — which is what creates the `vX.Y.Z` tag.

So the commit messages *are* the changelog. Types map to sections via
`release-please-config.json`; `feat` bumps the minor, `fix` (and everything else)
the patch, and `!` / `BREAKING CHANGE` also bumps the minor while the app is
pre-1.0 (`bump-minor-pre-major`).

## Key Dependencies

**Frontend**
- `vue` 3.5 — UI framework
- `@tauri-apps/api` 2 — IPC, events
- `@tauri-apps/plugin-dialog` — folder/file picker

**Backend (Rust)**
- `tauri` 2
- `tauri-plugin-dialog` — native file/folder dialogs
- `tauri-plugin-opener` — open files/URLs in default app
- `serde` / `serde_json` — serialization for IPC

## Conventions

### Frontend
- Single-file components (`.vue`) with `<script setup lang="ts">`
- Keep IPC calls in composables or a dedicated `src/api/` layer — not scattered in components
- Tauri commands are invoked via `@tauri-apps/api/core` `invoke()`

### Backend
- All Tauri commands go in `src-tauri/src/lib.rs` (or submodules imported there)
- Register new commands in the `invoke_handler!` macro in `lib.rs`
- Use `serde::Serialize` / `serde::Deserialize` on types crossing the IPC boundary
- Prefer returning `Result<T, String>` from commands so errors surface cleanly on the frontend

### IPC Pattern
```rust
// Rust
#[tauri::command]
fn my_command(arg: String) -> Result<MyType, String> { ... }
```
```ts
// Vue
import { invoke } from "@tauri-apps/api/core";
const result = await invoke<MyType>("my_command", { arg: "value" });
```

## File Capabilities

Tauri 2 uses a capability system (`src-tauri/capabilities/`). When adding new plugins or filesystem access, update `default.json` with the required permissions.
