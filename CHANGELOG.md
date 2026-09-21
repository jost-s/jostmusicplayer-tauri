# Changelog

## [0.2.0](https://github.com/jost-s/jostmusicplayer-tauri/compare/v0.1.0...v0.2.0) (2026-07-16)


### ⚠ BREAKING CHANGES

* show a column browser to filter and search field
* rebuild DB on schema mismatch

### Features

* add id3 tag editing ([1131a8a](https://github.com/jost-s/jostmusicplayer-tauri/commit/1131a8a83bdc4d450297846db4e039b6332cf86e))
* keep played back track focused when clearing filters ([4e489e9](https://github.com/jost-s/jostmusicplayer-tauri/commit/4e489e9465238f18530bce1dc76535e4cd86122e))
* add volume slider ([f44aa16](https://github.com/jost-s/jostmusicplayer-tauri/commit/f44aa166b9d33c89c88c1abbd8ced5794ccbd419))
* add support for media center and media keys ([ca3c7a9](https://github.com/jost-s/jostmusicplayer-tauri/commit/ca3c7a9cf179df3969cbd6433b8274595e7e216a))
* enable cancelling a folder scan ([d705de2](https://github.com/jost-s/jostmusicplayer-tauri/commit/d705de276a2767145831194cc56b79fa4eba23bc))
* **ui:** progressively show progress of scanning the file system ([2c2a772](https://github.com/jost-s/jostmusicplayer-tauri/commit/2c2a772ba44737e179f928ea811fba2537013df9))
* show a column browser to filter and search field ([b167c7c](https://github.com/jost-s/jostmusicplayer-tauri/commit/b167c7ca2a7fe00e4ac345a063dc9f9940e4802b))


### Bug Fixes

* reset media center metadata when last track ends ([4ce3abe](https://github.com/jost-s/jostmusicplayer-tauri/commit/4ce3abebc4dde26ccd89e8e67683fc08df5a1487))
* detect last track in playlist and stop after it ([62c31d9](https://github.com/jost-s/jostmusicplayer-tauri/commit/62c31d995019ad25654f5ae98af2180c5d8acf45))
* decode mp3 files that symphonia considers invalid anyway ([c04b00c](https://github.com/jost-s/jostmusicplayer-tauri/commit/c04b00cfae4703753b1ca871042e6884a6f32bb5))


### Refactoring

* swap volume and settings icon in footer ([50ed0fd](https://github.com/jost-s/jostmusicplayer-tauri/commit/50ed0fda80f7231bdc7cb307f74fb582aff022d5))
* rebuild DB on schema mismatch ([7abf7e3](https://github.com/jost-s/jostmusicplayer-tauri/commit/7abf7e3a12a064963edac54b4e812296275e1ce2))


### Chores

* bump version to 0.2.0 ([cd4beaf](https://github.com/jost-s/jostmusicplayer-tauri/commit/cd4beaf5aa2fc7a318f59a828aee9aa0a9a216a3))

## 0.1.0 (2026-06-22)


### Features

* scroll selected track into view when changing sort order ([faa907e](https://github.com/jost-s/jostmusicplayer-tauri/commit/faa907ea4436ac26592d0c1eb1823659ce954bea))
* play next track when one ends ([f484e77](https://github.com/jost-s/jostmusicplayer-tauri/commit/f484e77158097ff43ed264e0c8caeaa8b0e2152f))
* export logs to file and add menu item to access it ([4de7e9d](https://github.com/jost-s/jostmusicplayer-tauri/commit/4de7e9dad482c18c102729f13b19a33f212c1ed5))
* add support for aac files ([b30107e](https://github.com/jost-s/jostmusicplayer-tauri/commit/b30107eef414cbe3e2b8b0d17fff00a090f3a4d0))
* add support for opus files ([5a2adc6](https://github.com/jost-s/jostmusicplayer-tauri/commit/5a2adc6a2ee8ad84dce5c61b605f6dc45b5afcf0))
* add track playback ([ade3599](https://github.com/jost-s/jostmusicplayer-tauri/commit/ade359999582e47b07cd0b918a71a49597ba426e))
* add secondary sorting by track number ([575c937](https://github.com/jost-s/jostmusicplayer-tauri/commit/575c937c2cd4ec1cc370ee0285fa5319935e0414))
* display music library table ([dc98d5e](https://github.com/jost-s/jostmusicplayer-tauri/commit/dc98d5e0ca9539924e68335bb3c1a26c5a5957df))
* create tauri app with folder selection ([d438b89](https://github.com/jost-s/jostmusicplayer-tauri/commit/d438b89a24caba467d122e1c92723571d7d0709f))


### Refactoring

* **ui:** move folder selection into settings dialog ([2e1159d](https://github.com/jost-s/jostmusicplayer-tauri/commit/2e1159d70ac04272e1e117ac2b245070a9b91eab))
* virtualize track table ([bbd89cd](https://github.com/jost-s/jostmusicplayer-tauri/commit/bbd89cd3a2a34cb9713148b8990aa66d2e38c73e))


### Build System

* optimize release ([5aeb819](https://github.com/jost-s/jostmusicplayer-tauri/commit/5aeb8192a521d0b85f69d3dff5717a6f42324d53))


### Continuous Integration

* enable release workflow on dispatch ([b48d498](https://github.com/jost-s/jostmusicplayer-tauri/commit/b48d498003240c1b00965abe25bd0fc649377231))
* enable permissions to create a release ([fa2c0c4](https://github.com/jost-s/jostmusicplayer-tauri/commit/fa2c0c402d04a797b6acac3262ff4a2be7aea4a7))
* add cross-platform build dry-run workflow ([8bf4140](https://github.com/jost-s/jostmusicplayer-tauri/commit/8bf4140c9f775c13e672824fb46543065bab073c))
* add test & build & release workflows ([38a2541](https://github.com/jost-s/jostmusicplayer-tauri/commit/38a2541c75e1eedc12702c9e15cb6da42029f2d2))
