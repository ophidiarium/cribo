# Changelog

## [0.4.4](https://github.com/tinovyatkin/serpen/compare/v0.4.3...v0.4.4) (2025-06-05)


### Features

* **docs:** implement dual licensing for documentation ([#54](https://github.com/tinovyatkin/serpen/issues/54)) ([865ac4d](https://github.com/tinovyatkin/serpen/commit/865ac4d0efe5a771489e06a51a88326e154b1d71))
* implement uv-style hierarchical configuration system ([#51](https://github.com/tinovyatkin/serpen/issues/51)) ([396d669](https://github.com/tinovyatkin/serpen/commit/396d669d9c6694dc31d0a12889cb4c30f826584e))
* smart circular dependency resolution with comprehensive test coverage ([#56](https://github.com/tinovyatkin/serpen/issues/56)) ([0f609bd](https://github.com/tinovyatkin/serpen/commit/0f609bda02be7b480cdf386f59e5627bed40ad21))

## [0.4.3](https://github.com/tinovyatkin/serpen/compare/v0.4.2...v0.4.3) (2025-06-05)


### Bug Fixes

* **ci:** resolve npm package generation and commitlint config issues ([4c4b7ca](https://github.com/tinovyatkin/serpen/commit/4c4b7cae5d0ef4d7f97d7aa24c7e10ed23ddc32a))

## [0.4.2](https://github.com/tinovyatkin/serpen/compare/v0.4.1...v0.4.2) (2025-06-05)


### Features

* **ci:** add manual trigger support to release-please workflow ([3ac5648](https://github.com/tinovyatkin/serpen/commit/3ac5648ad15fe7d8ea0338a9d6b9237fdf1f1019))
* implement automated release management with conventional commits ([#47](https://github.com/tinovyatkin/serpen/issues/47)) ([5597fd4](https://github.com/tinovyatkin/serpen/commit/5597fd4c5d8963319751a3b7074cb1e92bbb9de9))
* migrate to ruff crates for parsing and AST ([#45](https://github.com/tinovyatkin/serpen/issues/45)) ([3b94d97](https://github.com/tinovyatkin/serpen/commit/3b94d977c6d91cc93bc784414a25c8ea58be82b7))
* **release:** add Aqua and UBI CLI installation support ([#49](https://github.com/tinovyatkin/serpen/issues/49)) ([eeb550f](https://github.com/tinovyatkin/serpen/commit/eeb550f6cf1eff6f0f10696fd255d7feac082045))
* **release:** include npm package.json version management in release-please ([73b5726](https://github.com/tinovyatkin/serpen/commit/73b57263626fbb184991ece37c18cfe8cc3d1310))


### Bug Fixes

* **ci:** add missing permissions and explicit command for release-please ([cf15ecd](https://github.com/tinovyatkin/serpen/commit/cf15ecda91704a2e255a44463431fec24fada935))
* **ci:** remove invalid command parameter from release-please action ([7b2dafa](https://github.com/tinovyatkin/serpen/commit/7b2dafa3b4ba8659352eef28cee2f972724c2f9f))
* **ci:** use PAT token and full git history for release-please ([92dedbe](https://github.com/tinovyatkin/serpen/commit/92dedbe959691fc092e9e1e8090507ed531ba1b0))
* **release:** configure release-please for Cargo workspace ([ef719cd](https://github.com/tinovyatkin/serpen/commit/ef719cddc35284750945248bc18fa53c63a86aad))
* **release:** reuse release-please version.txt in release workflow ([921f300](https://github.com/tinovyatkin/serpen/commit/921f3006ac88938cf67e40225e3e1f7eaa7c1c34))
