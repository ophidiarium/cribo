# Changelog

## [0.4.9](https://github.com/ophidiarium/cribo/compare/v0.4.8...v0.4.9) (2025-06-07)


### Bug Fixes

* **ci:** establish baseline benchmarks for performance tracking ([#77](https://github.com/ophidiarium/cribo/issues/77)) ([337d0f1](https://github.com/ophidiarium/cribo/commit/337d0f1c986a419f53333e22e7188c10a480dff0))
* **ci:** restore start-point parameters for proper PR benchmarking ([#79](https://github.com/ophidiarium/cribo/issues/79)) ([d826376](https://github.com/ophidiarium/cribo/commit/d826376d8b47f39e731d278b0de6292c84c136d0))

## [0.4.8](https://github.com/ophidiarium/cribo/compare/v0.4.7...v0.4.8) (2025-06-07)


### Features

* **ast:** handle relative imports from parent packages ([#70](https://github.com/ophidiarium/cribo/issues/70)) ([799790d](https://github.com/ophidiarium/cribo/commit/799790dea090549dc9863eca00ddc92ba04eb8ff))
* **ci:** add comprehensive benchmarking infrastructure ([#75](https://github.com/ophidiarium/cribo/issues/75)) ([e159b1f](https://github.com/ophidiarium/cribo/commit/e159b1fdbc34201044088b03d667e307e1d4cc82))

## [0.4.7](https://github.com/tinovyatkin/serpen/compare/v0.4.6...v0.4.7) (2025-06-06)


### Features

* integrate ruff linting for bundle output for cross-validation ([#66](https://github.com/tinovyatkin/serpen/issues/66)) ([170deda](https://github.com/tinovyatkin/serpen/commit/170deda60850f425d57647fb9ca88904f7f72a26))


### Bug Fixes

* **ci:** avoid double run of lint on PRs ([281289c](https://github.com/tinovyatkin/serpen/commit/281289ce97d508fe9541ae211f2c77c260d9e3ec))

## [0.4.6](https://github.com/tinovyatkin/serpen/compare/v0.4.5...v0.4.6) (2025-06-06)


### Features

* add comprehensive `from __future__` imports support with generic snapshot testing framework ([#63](https://github.com/tinovyatkin/serpen/issues/63)) ([e74c6e1](https://github.com/tinovyatkin/serpen/commit/e74c6e1275f6de9950cb8cc62a5771c743acb722))

## [0.4.5](https://github.com/tinovyatkin/serpen/compare/v0.4.4...v0.4.5) (2025-06-05)


### Bug Fixes

* resolve module import detection for aliased imports ([#57](https://github.com/tinovyatkin/serpen/issues/57)) ([95bc652](https://github.com/tinovyatkin/serpen/commit/95bc652c0a0e979abbed06a82654dfd7b7eddb52))

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
