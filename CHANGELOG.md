# Changelog

## [0.4.27](https://github.com/ophidiarium/cribo/compare/v0.4.26...v0.4.27) (2025-06-13)


### Bug Fixes

* **deps:** upgrade ruff crates from 0.11.12 to 0.11.13 ([#122](https://github.com/ophidiarium/cribo/issues/122)) ([878f73f](https://github.com/ophidiarium/cribo/commit/878f73f2486a4b2c5b6696231118633f96508b5c))

## [0.4.26](https://github.com/ophidiarium/cribo/compare/v0.4.25...v0.4.26) (2025-06-13)


### Bug Fixes

* **bundler:** resolve all fixable xfail import test cases ([#120](https://github.com/ophidiarium/cribo/issues/120)) ([2e3fd31](https://github.com/ophidiarium/cribo/commit/2e3fd31dfb42c9452567f67922ac704082bf6c11))

## [0.4.25](https://github.com/ophidiarium/cribo/compare/v0.4.24...v0.4.25) (2025-06-12)


### Features

* **bundler:** semantically aware bundler ([#118](https://github.com/ophidiarium/cribo/issues/118)) ([1314d3b](https://github.com/ophidiarium/cribo/commit/1314d3b034da76910c292332d084ee68eccab1ea))


### Bug Fixes

* **ai:** remove LSP recommendations ([dbf8f0b](https://github.com/ophidiarium/cribo/commit/dbf8f0bbd1be4921241865d1b50a45677b0f9166))

## [0.4.24](https://github.com/ophidiarium/cribo/compare/v0.4.23...v0.4.24) (2025-06-11)


### Features

* **bundler:** migrate unused imports trimmer to graph-based approach ([#115](https://github.com/ophidiarium/cribo/issues/115)) ([0098bb0](https://github.com/ophidiarium/cribo/commit/0098bb01ed166abc4dd2856e77530e303acac9ff))

## [0.4.23](https://github.com/ophidiarium/cribo/compare/v0.4.22...v0.4.23) (2025-06-11)


### Features

* **bundler:** ensure sys and types imports follow deterministic ordering ([#113](https://github.com/ophidiarium/cribo/issues/113)) ([73f6ea6](https://github.com/ophidiarium/cribo/commit/73f6ea6b5e6435d4e530c37f3dab2ecc7adbafe0))

## [0.4.22](https://github.com/ophidiarium/cribo/compare/v0.4.21...v0.4.22) (2025-06-11)


### Features

* **bundler:** integrate unused import trimming into static bundler ([#108](https://github.com/ophidiarium/cribo/issues/108)) ([b9473ff](https://github.com/ophidiarium/cribo/commit/b9473ff69aefe6bb5ec91b708a707cc19fa36c3e))


### Bug Fixes

* **bundler:** ensure future imports are correctly hoisted and late imports handled ([#112](https://github.com/ophidiarium/cribo/issues/112)) ([024b6d8](https://github.com/ophidiarium/cribo/commit/024b6d8b0ceb01e636bcd26f6c4cce2f7215b21d))

## [0.4.21](https://github.com/ophidiarium/cribo/compare/v0.4.20...v0.4.21) (2025-06-10)


### Features

* **bundler:** implement static bundling to eliminate runtime exec() calls ([#104](https://github.com/ophidiarium/cribo/issues/104)) ([d8f4912](https://github.com/ophidiarium/cribo/commit/d8f4912adb179947001f044dd9394a31f1302aa1))

## [0.4.20](https://github.com/ophidiarium/cribo/compare/v0.4.19...v0.4.20) (2025-06-09)


### Bug Fixes

* **ai:** improve changelog prompt and use cheaper model ([49d81e4](https://github.com/ophidiarium/cribo/commit/49d81e439878af6c7c837d0f992ea50b7350b0a3))
* **bundler:** resolve Python exec scoping and enable module import detection ([#97](https://github.com/ophidiarium/cribo/issues/97)) ([e22a871](https://github.com/ophidiarium/cribo/commit/e22a8719584fa3bef4e563788fdd2825c2dd6c15))

## [0.4.19](https://github.com/ophidiarium/cribo/compare/v0.4.18...v0.4.19) (2025-06-09)


### Bug Fixes

* adjust OpenAI API curl ([da3922b](https://github.com/ophidiarium/cribo/commit/da3922bf37ff9b031c43ecfa72039ab73fcf855b))

## [0.4.18](https://github.com/ophidiarium/cribo/compare/v0.4.17...v0.4.18) (2025-06-09)


### Bug Fixes

* adjust OpenAI API curling ([1a5ddda](https://github.com/ophidiarium/cribo/commit/1a5ddda10249578407e09d3e15194d58606022fb))

## [0.4.17](https://github.com/ophidiarium/cribo/compare/v0.4.16...v0.4.17) (2025-06-09)


### Bug Fixes

* remove win32-ia32 ([0999927](https://github.com/ophidiarium/cribo/commit/09999273c411c878901dd1aadd3e4aa5ba9ec1b9))
* use curl to call OpenAI API ([f690f9f](https://github.com/ophidiarium/cribo/commit/f690f9fc7bacde34d30b483fab6d0ce041e716a0))

## [0.4.16](https://github.com/ophidiarium/cribo/compare/v0.4.15...v0.4.16) (2025-06-08)


### Bug Fixes

* **ci:** add missing TAG reference ([2ffe264](https://github.com/ophidiarium/cribo/commit/2ffe264d22deb4f965d140ff4429d5b110934251))

## [0.4.15](https://github.com/ophidiarium/cribo/compare/v0.4.14...v0.4.15) (2025-06-08)


### Bug Fixes

* **ci:** use --quiet for codex ([ca14208](https://github.com/ophidiarium/cribo/commit/ca1420890eb3b0b0abf9aa573554daa1c53ad978))

## [0.4.14](https://github.com/ophidiarium/cribo/compare/v0.4.13...v0.4.14) (2025-06-08)


### Bug Fixes

* **ci:** missing -r for jq ([31c0ffd](https://github.com/ophidiarium/cribo/commit/31c0ffdcb603aace20056e9f1e5c8f1708c1abac))

## [0.4.13](https://github.com/ophidiarium/cribo/compare/v0.4.12...v0.4.13) (2025-06-08)


### Features

* **cli:** add stdout output mode for debugging workflows ([#87](https://github.com/ophidiarium/cribo/issues/87)) ([34a89e9](https://github.com/ophidiarium/cribo/commit/34a89e9763e40b1f4922402ca93f85e68b7883f6))


### Bug Fixes

* **ci:** serpen leftovers ([e1acaed](https://github.com/ophidiarium/cribo/commit/e1acaedec3373849398d3aa71d9cccaae2db3609))
* serpen leftovers ([6366453](https://github.com/ophidiarium/cribo/commit/6366453a07a893b2c0ae3b92235b28028d7ba1be))
* serpen leftovers ([5aa2a64](https://github.com/ophidiarium/cribo/commit/5aa2a6420fa012bd303ed3f12ae5d712d1b05748))

## [0.4.12](https://github.com/ophidiarium/cribo/compare/v0.4.11...v0.4.12) (2025-06-08)


### Features

* **cli:** add verbose flag repetition support for progressive debugging ([#85](https://github.com/ophidiarium/cribo/issues/85)) ([cc845e0](https://github.com/ophidiarium/cribo/commit/cc845e03f2fa0d70eb69dcf2e30b600ed5a5b38a))

## [0.4.11](https://github.com/ophidiarium/cribo/compare/v0.4.10...v0.4.11) (2025-06-08)


### Features

* **ai:** add AI powered release not summary ([6df72c6](https://github.com/ophidiarium/cribo/commit/6df72c66f179dded1bd098fad0ca923daf49dd48))


### Bug Fixes

* **bundler:** re-enable package init test and fix parent package imports ([#83](https://github.com/ophidiarium/cribo/issues/83)) ([83856b3](https://github.com/ophidiarium/cribo/commit/83856b3a4036df75ed9999f65b0738142ab07000))

## [0.4.10](https://github.com/ophidiarium/cribo/compare/v0.4.9...v0.4.10) (2025-06-07)


### Features

* **test:** re-enable single dot relative import test ([#80](https://github.com/ophidiarium/cribo/issues/80)) ([f698072](https://github.com/ophidiarium/cribo/commit/f6980728850b4305000c2dda46049074f413ce02))

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
