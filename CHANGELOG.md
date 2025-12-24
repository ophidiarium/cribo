# Changelog

## [0.8.1](https://github.com/ophidiarium/cribo/compare/v0.8.0...v0.8.1) (2025-12-24)


### Bug Fixes

* keep PyPI attestations ([6e6b77b](https://github.com/ophidiarium/cribo/commit/6e6b77bf1c91495aff0522e94d9257342eb343ef))
* repack aarch64 wheel before PyPI ([5921b80](https://github.com/ophidiarium/cribo/commit/5921b803d96a7ef9507273d5e58de946b8e79293))

## [0.8.0](https://github.com/ophidiarium/cribo/compare/v0.7.0...v0.8.0) (2025-12-24)


### Features

* add __qualname__ ([84bce8d](https://github.com/ophidiarium/cribo/commit/84bce8d9b6f61518bd84a370e7891f77604c34e6))
* add bun and dprint installation to copilot setup workflow ([#287](https://github.com/ophidiarium/cribo/issues/287)) ([751c19c](https://github.com/ophidiarium/cribo/commit/751c19cc329527401373a8ad6c52187fea89f6da))
* add Claude Code hook to prevent direct commits to main branch ([8df0a68](https://github.com/ophidiarium/cribo/commit/8df0a6811625b197029ea5fe8575120568cd855c))
* add comprehensive `from __future__` imports support with generic snapshot testing framework ([#63](https://github.com/ophidiarium/cribo/issues/63)) ([5612dd9](https://github.com/ophidiarium/cribo/commit/5612dd9bab5092d0f73319c8aa7a8bebda7b514b))
* add httpx to ecosystem tests and type_checking_imports fixture ([#306](https://github.com/ophidiarium/cribo/issues/306)) ([dc382e7](https://github.com/ophidiarium/cribo/commit/dc382e7d6f1d273979b2561013cb09e6e047ff8b))
* add idna to ecosystem tests with improved test infrastructure ([#372](https://github.com/ophidiarium/cribo/issues/372)) ([6344432](https://github.com/ophidiarium/cribo/commit/6344432f131e40c2ee1e884cf9d88bd4246d5fd4))
* add VIRTUAL_ENV support ([#40](https://github.com/ophidiarium/cribo/issues/40)) ([70ace41](https://github.com/ophidiarium/cribo/commit/70ace419b1a872c0eb3dd7970206f1e19e73c60b))
* **ai:** add AI powered release not summary ([eaf4b2b](https://github.com/ophidiarium/cribo/commit/eaf4b2b27bdc5ac992da401a70b803a475821f4a))
* AST rewrite ([#42](https://github.com/ophidiarium/cribo/issues/42)) ([1172b20](https://github.com/ophidiarium/cribo/commit/1172b202b7170529977f536a39f88a3d91437311))
* **ast:** handle relative imports from parent packages ([#70](https://github.com/ophidiarium/cribo/issues/70)) ([751ac07](https://github.com/ophidiarium/cribo/commit/751ac0708ac2e0991e24ac121ea8b157d956a034))
* **bundler:** ensure sys and types imports follow deterministic ordering ([#113](https://github.com/ophidiarium/cribo/issues/113)) ([05738ee](https://github.com/ophidiarium/cribo/commit/05738ee4aa81de6e424de9aa276db0433c850d23))
* **bundler:** implement static bundling to eliminate runtime exec() calls ([#104](https://github.com/ophidiarium/cribo/issues/104)) ([fceb34f](https://github.com/ophidiarium/cribo/commit/fceb34f3af1c6ef7fc63aee446c7e6481248f003))
* **bundler:** integrate unused import trimming into static bundler ([#108](https://github.com/ophidiarium/cribo/issues/108)) ([d745a4f](https://github.com/ophidiarium/cribo/commit/d745a4ff7ab609f74ad96c7e878e7db41bd1c687))
* **bundler:** migrate unused imports trimmer to graph-based approach ([#115](https://github.com/ophidiarium/cribo/issues/115)) ([5e94496](https://github.com/ophidiarium/cribo/commit/5e9449642c5a2aa5dd12d687d4842bfb2e7a4c1f))
* **bundler:** semantically aware bundler ([#118](https://github.com/ophidiarium/cribo/issues/118)) ([d0be9f8](https://github.com/ophidiarium/cribo/commit/d0be9f8c7883b7369589cd422a5408c47e7a69ee))
* centralize __init__ and __main__ handling via python module ([#349](https://github.com/ophidiarium/cribo/issues/349)) ([9c3a79c](https://github.com/ophidiarium/cribo/commit/9c3a79c43d2c13a1ba7573fbccedb3ccc92be29f))
* **ci:** add comprehensive benchmarking infrastructure ([#75](https://github.com/ophidiarium/cribo/issues/75)) ([ac7a92e](https://github.com/ophidiarium/cribo/commit/ac7a92e001db908c2a000de50f6b65a485886e9e))
* **ci:** add manual trigger support to release-please workflow ([162e501](https://github.com/ophidiarium/cribo/commit/162e50187d62f19a90e8956138488b8e8f0b1683))
* **ci:** add rust-code-analysis-cli ([43523e1](https://github.com/ophidiarium/cribo/commit/43523e1c617949a66f73d63e42bf73c6ce1eb2fe))
* **ci:** only show rust analyzer for changed files ([#132](https://github.com/ophidiarium/cribo/issues/132)) ([de7a875](https://github.com/ophidiarium/cribo/commit/de7a875944acfe9a20219a2da020a8cace7dfd2a))
* **cli:** add stdout output mode for debugging workflows ([#87](https://github.com/ophidiarium/cribo/issues/87)) ([f020c46](https://github.com/ophidiarium/cribo/commit/f020c46eed0bf208056ebf09d242feecb895c975))
* **cli:** add verbose flag repetition support for progressive debugging ([#85](https://github.com/ophidiarium/cribo/issues/85)) ([2fb6941](https://github.com/ophidiarium/cribo/commit/2fb6941b54854e8bdaad51050eaf729adcbbb4f6))
* **docs:** implement dual licensing for documentation ([#54](https://github.com/ophidiarium/cribo/issues/54)) ([24ad056](https://github.com/ophidiarium/cribo/commit/24ad056e086ec5947dc0e0d34faf2031e4e9d150))
* ecosystem tests foundation ([#163](https://github.com/ophidiarium/cribo/issues/163)) ([6695093](https://github.com/ophidiarium/cribo/commit/6695093c80da34d76d2227085bbf33920213d1d3))
* enhance circular dependency detection and prepare for import rewriting ([#126](https://github.com/ophidiarium/cribo/issues/126)) ([6a84d5f](https://github.com/ophidiarium/cribo/commit/6a84d5fa3bbbeba465bd25a59be37505f5d4bb18))
* implement AST visitor pattern for comprehensive import discovery ([#130](https://github.com/ophidiarium/cribo/issues/130)) ([a890b86](https://github.com/ophidiarium/cribo/commit/a890b864342ab421004a2aa3c95da97e2a389406))
* implement automated release management with conventional commits ([#47](https://github.com/ophidiarium/cribo/issues/47)) ([bfd7583](https://github.com/ophidiarium/cribo/commit/bfd7583fa607cd62c22c406fd33d75e8cced8cb7))
* implement centralized namespace management system ([#263](https://github.com/ophidiarium/cribo/issues/263)) ([c9b19f0](https://github.com/ophidiarium/cribo/commit/c9b19f0f209abd9646e2438dbfe0bf91306d7539))
* implement static importlib support and file deduplication ([#157](https://github.com/ophidiarium/cribo/issues/157)) ([77b6609](https://github.com/ophidiarium/cribo/commit/77b6609f99b9767cb6bb0202f1bba13bb61df441))
* implement tree-shaking to remove unused code and imports ([#152](https://github.com/ophidiarium/cribo/issues/152)) ([26c2933](https://github.com/ophidiarium/cribo/commit/26c2933e291f640091fb53cfd5e1e1f8db603ab4))
* implement uv-style hierarchical configuration system ([#51](https://github.com/ophidiarium/cribo/issues/51)) ([bf11be0](https://github.com/ophidiarium/cribo/commit/bf11be0a54d90330f2d645f97cfee28a2eaab4d8))
* integrate Bencher.dev for ecosystem bundling metrics ([#379](https://github.com/ophidiarium/cribo/issues/379)) ([25c293c](https://github.com/ophidiarium/cribo/commit/25c293c14a0cb3e2d387390350215181825f34be))
* integrate ruff linting for bundle output for cross-validation ([#66](https://github.com/ophidiarium/cribo/issues/66)) ([4aff4cc](https://github.com/ophidiarium/cribo/commit/4aff4cc531d731901defb1c6d09d8bf4d8c07045))
* migrate to ruff crates for parsing and AST ([#45](https://github.com/ophidiarium/cribo/issues/45)) ([eab82e3](https://github.com/ophidiarium/cribo/commit/eab82e3f45ff5eaf90a53462f8a264d807408dc9))
* post-checkout hooks ([9063ae2](https://github.com/ophidiarium/cribo/commit/9063ae2b613c1b355440ceb1237d8858fa7390d5))
* **release:** add Aqua and UBI CLI installation support ([#49](https://github.com/ophidiarium/cribo/issues/49)) ([461dc7c](https://github.com/ophidiarium/cribo/commit/461dc7c6a70cbea2611914cba6279bd8d5433534))
* **release:** include npm package.json version management in release-please ([a8fee6f](https://github.com/ophidiarium/cribo/commit/a8fee6faa57bd39448419e560126867d27fc8443))
* smart circular dependency resolution with comprehensive test coverage ([#56](https://github.com/ophidiarium/cribo/issues/56)) ([46f8404](https://github.com/ophidiarium/cribo/commit/46f8404c8f4811bb88a407a53021dc2c92a2d455))
* **test:** enhance snapshot framework with YAML requirements and third-party import support ([#134](https://github.com/ophidiarium/cribo/issues/134)) ([6770e0b](https://github.com/ophidiarium/cribo/commit/6770e0b86ef2c4e06315aabd69ca8bbb2a00a031))
* **test:** re-enable single dot relative import test ([#80](https://github.com/ophidiarium/cribo/issues/80)) ([42d42d8](https://github.com/ophidiarium/cribo/commit/42d42d812c835ea2fef7bb72dfbc820f1aac6590))
* use taplo and stable rust ([8629456](https://github.com/ophidiarium/cribo/commit/86294565102f23b31e1f3f1fcd935c6b5ab02b08))


### Bug Fixes

* add module namespace assignments for wildcard imports in wrapper inits ([#318](https://github.com/ophidiarium/cribo/issues/318)) ([553ef44](https://github.com/ophidiarium/cribo/commit/553ef4400c901192a4de8a8b2d208ec6c52d1544))
* address review comments from PR [#372](https://github.com/ophidiarium/cribo/issues/372) ([#377](https://github.com/ophidiarium/cribo/issues/377)) ([632e9b7](https://github.com/ophidiarium/cribo/commit/632e9b755aed9fe65eb7b4d79a9410b37b408185))
* adjust OpenAI API curl ([26ef069](https://github.com/ophidiarium/cribo/commit/26ef069e2ed6aa002a53f17fb1cb44da5d4e84f7))
* adjust OpenAI API curling ([303e5ad](https://github.com/ophidiarium/cribo/commit/303e5adb921ed82798088cd124482c74d1cb2fef))
* **ai:** improve changelog prompt and use cheaper model ([052fb78](https://github.com/ophidiarium/cribo/commit/052fb782f472c48f6837f22217c509eed2858e6e))
* **ai:** remove LSP recommendations ([9ca7676](https://github.com/ophidiarium/cribo/commit/9ca7676bb5e04659256385a7017c3d6c0325a311))
* apply renames to metaclass keyword arguments in class definitions ([#295](https://github.com/ophidiarium/cribo/issues/295)) ([01f2e1a](https://github.com/ophidiarium/cribo/commit/01f2e1a817f164d14898617577d48617dcda41bb))
* assign init function results to modules in sorted initialization ([#222](https://github.com/ophidiarium/cribo/issues/222)) ([0438108](https://github.com/ophidiarium/cribo/commit/043810880722249783e3a0c0f0fa978d9f77f99c))
* attach entry module exports to namespace for package imports ([#366](https://github.com/ophidiarium/cribo/issues/366)) ([917cc4d](https://github.com/ophidiarium/cribo/commit/917cc4d1b5b340ddf50ac683dae31b959a5b0511))
* base branch bench missed feature ([011ffcd](https://github.com/ophidiarium/cribo/commit/011ffcd91407d5ff1bd806ea2aeb944a8f6e6c8b))
* bencher install ([431ae61](https://github.com/ophidiarium/cribo/commit/431ae61511af683fec2e2988168de27cc55d16ae))
* **bundler:** apply symbol renames to class base classes during inheritance ([#188](https://github.com/ophidiarium/cribo/issues/188)) ([fa2adc4](https://github.com/ophidiarium/cribo/commit/fa2adc42d012c983d04a09a9b1dbce4232148978))
* **bundler:** apply symbol renames to class base classes during inheritance ([#189](https://github.com/ophidiarium/cribo/issues/189)) ([909bd09](https://github.com/ophidiarium/cribo/commit/909bd09dfe53389817bf4651822c940760f7e2df))
* **bundler:** ensure future imports are correctly hoisted and late imports handled ([#112](https://github.com/ophidiarium/cribo/issues/112)) ([1e8ac71](https://github.com/ophidiarium/cribo/commit/1e8ac71f10a89d927d2586c9759f132b1ca0a70d))
* **bundler:** handle __version__ export and eliminate duplicate module assignments ([#213](https://github.com/ophidiarium/cribo/issues/213)) ([9c62e03](https://github.com/ophidiarium/cribo/commit/9c62e03a3a0d1c04c2da35b645297b63efdf3d5e))
* **bundler:** handle circular dependencies with module-level attribute access ([924b9f1](https://github.com/ophidiarium/cribo/commit/924b9f1163587b7b63af706bfab063cd34afd327))
* **bundler:** handle circular dependencies with module-level attribute access ([#219](https://github.com/ophidiarium/cribo/issues/219)) ([ffb49c0](https://github.com/ophidiarium/cribo/commit/ffb49c041c1b04d8e6ac92be41820ce1b10c2255))
* **bundler:** handle conditional imports in if/else and try/except blocks ([#184](https://github.com/ophidiarium/cribo/issues/184)) ([de37ad2](https://github.com/ophidiarium/cribo/commit/de37ad22b4648959ccf146bd382de00fdfca0931))
* **bundler:** preserve import aliases and prevent duplication in hoisted imports ([#135](https://github.com/ophidiarium/cribo/issues/135)) ([632658e](https://github.com/ophidiarium/cribo/commit/632658e19375496b8cce47545802b006d1f5a9bd))
* **bundler:** prevent duplicate namespace assignments when processing parent modules ([#216](https://github.com/ophidiarium/cribo/issues/216)) ([3bcf2a4](https://github.com/ophidiarium/cribo/commit/3bcf2a45eb99992d47f531b3b23c7395d54c2e8c))
* **bundler:** prevent transformation of Python builtins to module attributes ([#212](https://github.com/ophidiarium/cribo/issues/212)) ([1a9b7a9](https://github.com/ophidiarium/cribo/commit/1a9b7a9592a3a5016db30847eb85246c79207c71))
* **bundler:** re-enable package init test and fix parent package imports ([#83](https://github.com/ophidiarium/cribo/issues/83)) ([b352fa4](https://github.com/ophidiarium/cribo/commit/b352fa45a6c4aada2c831b60b03d832f4b6129f9))
* **bundler:** resolve all fixable xfail import test cases ([#120](https://github.com/ophidiarium/cribo/issues/120)) ([bad94a2](https://github.com/ophidiarium/cribo/commit/bad94a230bbd88db10535dade9e483b6ff9bf1e7))
* **bundler:** resolve forward reference issues in cross-module dependencies ([#197](https://github.com/ophidiarium/cribo/issues/197)) ([41b633d](https://github.com/ophidiarium/cribo/commit/41b633d93ffcc413fa0f0e81d2acefd27dcd1fca))
* **bundler:** resolve Python exec scoping and enable module import detection ([#97](https://github.com/ophidiarium/cribo/issues/97)) ([5dab748](https://github.com/ophidiarium/cribo/commit/5dab7489f63549c9900d183477482b02b80dd3e3))
* **bundler:** skip import assignments for tree-shaken symbols ([#214](https://github.com/ophidiarium/cribo/issues/214)) ([6827e4c](https://github.com/ophidiarium/cribo/commit/6827e4c06b8c78ceed61c0000f42951ad6439000))
* **bundler:** wrap modules in circular deps that access imported attributes ([#218](https://github.com/ophidiarium/cribo/issues/218)) ([3f0b093](https://github.com/ophidiarium/cribo/commit/3f0b0934f4fa5eaf7d8f480e9146a35802e52786))
* centralize namespace management to prevent duplicates and fix special module handling ([#261](https://github.com/ophidiarium/cribo/issues/261)) ([c23e8d2](https://github.com/ophidiarium/cribo/commit/c23e8d219cb4e39c398fd4ada330776841dc8ae8))
* **ci:** add missing permissions and explicit command for release-please ([f12f537](https://github.com/ophidiarium/cribo/commit/f12f537702c16ca86a713e2120dea100eb4e62b7))
* **ci:** add missing TAG reference ([5b5bbf6](https://github.com/ophidiarium/cribo/commit/5b5bbf6fa09b433c6f597c98710006d3887091ac))
* **ci:** avoid double run of lint on PRs ([d43dc2a](https://github.com/ophidiarium/cribo/commit/d43dc2acb7618e3d62d49d924e85c37fdd5cd03c))
* **ci:** establish baseline benchmarks for performance tracking ([#77](https://github.com/ophidiarium/cribo/issues/77)) ([98f1385](https://github.com/ophidiarium/cribo/commit/98f1385f2bef1ee19075a6562a20504c07e86b56))
* **ci:** missing -r for jq ([41472ed](https://github.com/ophidiarium/cribo/commit/41472eda758010e6b385daf92936c599f60e6ca2))
* **ci:** remove invalid command parameter from release-please action ([bd171c9](https://github.com/ophidiarium/cribo/commit/bd171c99dad177653726a14dfc8d326e1985d073))
* **ci:** resolve npm package generation and commitlint config issues ([dd70499](https://github.com/ophidiarium/cribo/commit/dd704995b99c6ee17a90a59e1adfa5778b1e9d98))
* **ci:** restore start-point parameters for proper PR benchmarking ([#79](https://github.com/ophidiarium/cribo/issues/79)) ([52a2a2a](https://github.com/ophidiarium/cribo/commit/52a2a2a69c86695e4b5f6507d1960353c82a1ba2))
* **ci:** serpen leftovers ([6d4d3c5](https://github.com/ophidiarium/cribo/commit/6d4d3c585376467f3e4f4d35dc4659995ff6adb3))
* **ci:** use --quiet for codex ([74e876a](https://github.com/ophidiarium/cribo/commit/74e876ae81371d7210c762ccb489481bc539c4b0))
* **ci:** use PAT token and full git history for release-please ([4203d39](https://github.com/ophidiarium/cribo/commit/4203d391b871992bfbeba8054be8b1ad1505e0a3))
* collect dependencies from nested classes and functions in graph builder ([#272](https://github.com/ophidiarium/cribo/issues/272)) ([b021ae1](https://github.com/ophidiarium/cribo/commit/b021ae194d8b5e9b1467fd593c98a99bd6887cc3))
* copilot setup steps ([3e1ecd0](https://github.com/ophidiarium/cribo/commit/3e1ecd09a907593875246ce05dc66f9b52930ceb))
* correctly reference symbols from wrapper modules in namespace assignments ([#298](https://github.com/ophidiarium/cribo/issues/298)) ([91e76e7](https://github.com/ophidiarium/cribo/commit/91e76e7bdc3266eb17bb4e2f27ef53fbfcd267d0))
* **deps:** upgrade ruff crates from 0.11.12 to 0.11.13 ([#122](https://github.com/ophidiarium/cribo/issues/122)) ([9e6c02d](https://github.com/ophidiarium/cribo/commit/9e6c02dd8be2ee58fc55f0555e27eba7413404d1))
* ecosystem testing testing advances ([#165](https://github.com/ophidiarium/cribo/issues/165)) ([a4db95f](https://github.com/ophidiarium/cribo/commit/a4db95fec312341f9247075e332f5b898cd84698))
* ensure private symbols imported by other modules are exported ([#328](https://github.com/ophidiarium/cribo/issues/328)) ([0f467ea](https://github.com/ophidiarium/cribo/commit/0f467ea700feae19efe4c161c5d62d69e1c596f4))
* ensure tree-shaking preserves imports within used functions and classes ([#330](https://github.com/ophidiarium/cribo/issues/330)) ([085695c](https://github.com/ophidiarium/cribo/commit/085695c67aaa2c5a90b59c6b7251e25068055a7b))
* handle built-in type re-exports correctly in bundled output ([#240](https://github.com/ophidiarium/cribo/issues/240)) ([4ab4e83](https://github.com/ophidiarium/cribo/commit/4ab4e83bda56de7ae5b2741a30a8a2a9a2f4a681))
* handle circular dependencies with __version__ module imports ([#314](https://github.com/ophidiarium/cribo/issues/314)) ([c2512e9](https://github.com/ophidiarium/cribo/commit/c2512e98cb232999be11c5e5edb6ed1f82c3da13))
* handle circular dependencies with stdlib-conflicting module names ([#281](https://github.com/ophidiarium/cribo/issues/281)) ([6ea838e](https://github.com/ophidiarium/cribo/commit/6ea838eb24ae2be2b8be73ecc04abb034d033408))
* handle circular imports from parent __init__ modules ([#362](https://github.com/ophidiarium/cribo/issues/362)) ([a8cc60f](https://github.com/ophidiarium/cribo/commit/a8cc60f3b870ba7fef220a4293aa95a687394c4b))
* handle lifted globals correctly in module transformation ([#325](https://github.com/ophidiarium/cribo/issues/325)) ([ff3ae54](https://github.com/ophidiarium/cribo/commit/ff3ae546f75bf2c6e92de0344cdc80cb7e8b8509))
* handle locals() calls in wrapped modules by static analysis ([#308](https://github.com/ophidiarium/cribo/issues/308)) ([c502713](https://github.com/ophidiarium/cribo/commit/c5027135970a73912bab711e384a81cf391cbda5))
* handle metaclass dependencies in class ordering ([1c67f3e](https://github.com/ophidiarium/cribo/commit/1c67f3ef80a742c24906e8434b03da2def0b1d44))
* handle relative imports in wrapper module init functions ([#356](https://github.com/ophidiarium/cribo/issues/356)) ([4c7dfb4](https://github.com/ophidiarium/cribo/commit/4c7dfb4bf17894b041568525075541e6834ecdfd))
* handle stdlib module name conflicts in bundler ([#279](https://github.com/ophidiarium/cribo/issues/279)) ([c800b32](https://github.com/ophidiarium/cribo/commit/c800b32e8c282361a526b8f79d9fe38492385bd4))
* handle submodules in __all__ exports correctly ([8b14937](https://github.com/ophidiarium/cribo/commit/8b14937d16f8f81766f2b727b2336e398dfba3ce))
* handle submodules in __all__ exports correctly ([#226](https://github.com/ophidiarium/cribo/issues/226)) ([b09bce3](https://github.com/ophidiarium/cribo/commit/b09bce3c9e50a0f8d13e8f1d175880439cf1996b))
* handle wildcard imports correctly for wrapper and inlined modules ([#294](https://github.com/ophidiarium/cribo/issues/294)) ([26d5617](https://github.com/ophidiarium/cribo/commit/26d56173a84ccac7d1f3156feb882da526a2a2da))
* handle wildcard imports from inlined modules that re-export wrapper module symbols ([#311](https://github.com/ophidiarium/cribo/issues/311)) ([940f275](https://github.com/ophidiarium/cribo/commit/940f27585f7d76ce6901acc8ad5a3910968ab04a))
* handle wildcard imports in wrapper modules with setattr pattern ([#310](https://github.com/ophidiarium/cribo/issues/310)) ([4db103a](https://github.com/ophidiarium/cribo/commit/4db103a58d949af1b79426c9d371557761e4505c))
* handle wrapper module imports in function default parameters ([#329](https://github.com/ophidiarium/cribo/issues/329)) ([2b4f1bc](https://github.com/ophidiarium/cribo/commit/2b4f1bc34531a142942b3372868f6a8ec2384dd3))
* implement function-scoped import rewriting for circular dependency resolution ([8cc923f](https://github.com/ophidiarium/cribo/commit/8cc923fd6809df0eb0a1b9a4df35d888c0df7bba)), closes [#128](https://github.com/ophidiarium/cribo/issues/128)
* improve class dependency ordering for metaclass and class body references ([#327](https://github.com/ophidiarium/cribo/issues/327)) ([e062df7](https://github.com/ophidiarium/cribo/commit/e062df7162db5bc76bbee4aba697a8c3984b7082))
* improve class ordering for cross-module inheritance ([#277](https://github.com/ophidiarium/cribo/issues/277)) ([392b42a](https://github.com/ophidiarium/cribo/commit/392b42ae535f8842ecd220c7be5e14da8796a734))
* include all module-scope symbols in namespace to support private imports ([#225](https://github.com/ophidiarium/cribo/issues/225)) ([77b77a5](https://github.com/ophidiarium/cribo/commit/77b77a50478d7b05eb7983311ecf6cc1ff5d3d06))
* include explicitly imported private symbols in circular dependencies ([#312](https://github.com/ophidiarium/cribo/issues/312)) ([94e7913](https://github.com/ophidiarium/cribo/commit/94e7913cbef55330c17688a595bd1161ed544af8))
* initialize wrapper modules for lazy imports in inlined modules ([#289](https://github.com/ophidiarium/cribo/issues/289)) ([2db1459](https://github.com/ophidiarium/cribo/commit/2db1459e17368f576c27001295e759c262c733a4))
* install msbuild on windows ([92ffaac](https://github.com/ophidiarium/cribo/commit/92ffaacd7756113c43528f95224b7bd823f2f4e2))
* major ast-rewriter improvement ([#43](https://github.com/ophidiarium/cribo/issues/43)) ([6a71aba](https://github.com/ophidiarium/cribo/commit/6a71aba0eb1df82d15216f103f7fdf8280ecd14f))
* prefer __init__.py over __main__.py for directory entry points ([#364](https://github.com/ophidiarium/cribo/issues/364)) ([2c1e6a9](https://github.com/ophidiarium/cribo/commit/2c1e6a94a910185eec8d7db8c2c33c0992708116))
* preserve aliased imports accessed via module attributes during tree-shaking ([#301](https://github.com/ophidiarium/cribo/issues/301)) ([30916f4](https://github.com/ophidiarium/cribo/commit/30916f4ff5da0b3ea4fc84c8701340f2b65638a7))
* preserve module docstrings in bundled output ([#386](https://github.com/ophidiarium/cribo/issues/386)) ([248a3f0](https://github.com/ophidiarium/cribo/commit/248a3f0e752e3a05bae9a9f71d99658d75b3d833))
* preserve stdlib imports and fix module initialization order for wrapper modules ([#283](https://github.com/ophidiarium/cribo/issues/283)) ([6201f22](https://github.com/ophidiarium/cribo/commit/6201f22d079097a62ca3fc537dfa4a1b16ea80c1))
* preserve symbols accessed dynamically via locals/globals with __all__ ([#317](https://github.com/ophidiarium/cribo/issues/317)) ([fafc7e4](https://github.com/ophidiarium/cribo/commit/fafc7e4bcb86ab788b30db2031cc20088412deca))
* prevent code generator from referencing tree-shaken symbols ([#305](https://github.com/ophidiarium/cribo/issues/305)) ([b8672c4](https://github.com/ophidiarium/cribo/commit/b8672c4175bf085451487be08b083d622f03ba7b))
* prevent globals() transformation in functions within circular dependency modules ([#368](https://github.com/ophidiarium/cribo/issues/368)) ([0e6baea](https://github.com/ophidiarium/cribo/commit/0e6baeaff3b28ce70aec65aaba043e4ab895a634))
* prevent stdlib module name conflicts in bundled imports ([#275](https://github.com/ophidiarium/cribo/issues/275)) ([59b9266](https://github.com/ophidiarium/cribo/commit/59b92662b908e2035dc7e0bb22305f774e0fe795))
* regenerate lockfile ([b036c9c](https://github.com/ophidiarium/cribo/commit/b036c9cf37f5de751cfaf2788ba4e7304ef42a39))
* relative imports being incorrectly classified as stdlib imports ([#267](https://github.com/ophidiarium/cribo/issues/267)) ([a703f0e](https://github.com/ophidiarium/cribo/commit/a703f0e82060a7b59e6ac0c1947867b1217673dd))
* release, again ([77d047b](https://github.com/ophidiarium/cribo/commit/77d047b3f6e3886ed91f74fb450db2c83804bcf7))
* **release:** configure release-please for Cargo workspace ([37a1f74](https://github.com/ophidiarium/cribo/commit/37a1f749f6f3631f804c04d70fdc7fdeecf251b8))
* **release:** reuse release-please version.txt in release workflow ([cbf84f4](https://github.com/ophidiarium/cribo/commit/cbf84f41d3a5fa9c1457f1a9be38146c323ec771))
* remove hardcoded http.cookiejar handling with generic submodule import solution ([#331](https://github.com/ophidiarium/cribo/issues/331)) ([750086d](https://github.com/ophidiarium/cribo/commit/750086d9e971994d7c451f670e4aee89361ecf0d))
* remove unnecessary statement reordering and self-referential wildcard imports ([9e1c8ae](https://github.com/ophidiarium/cribo/commit/9e1c8aee22802a4732b5e903ebadf9778c36d7e3))
* remove unused code to resolve clippy warnings ([bd2ace6](https://github.com/ophidiarium/cribo/commit/bd2ace603ff2142679c4211bf81b342b8729254e))
* remove win32-ia32 ([d6dab74](https://github.com/ophidiarium/cribo/commit/d6dab7488703c4a855b36b23d9a8c574a02e14a5))
* rename bundled_exit_code to python_exit_code for clarity ([72f3846](https://github.com/ophidiarium/cribo/commit/72f38463fc00d932b32389f02558ab8a67375ec6))
* replace cast with try_from for leading_dots conversion ([cea9082](https://github.com/ophidiarium/cribo/commit/cea9082c047e970d7e19f9c4e4a93c0d6b943569))
* replace unnecessary Debug formatting with Display for paths ([#260](https://github.com/ophidiarium/cribo/issues/260)) ([e4c6357](https://github.com/ophidiarium/cribo/commit/e4c6357e412c69c115740c7f6f3764e789cd9cdc))
* replacing broken inline python script ([e7854aa](https://github.com/ophidiarium/cribo/commit/e7854aa9d422d564fcde18de586099a66c5fa33f))
* resolve __all__ completely statically ([#247](https://github.com/ophidiarium/cribo/issues/247)) ([fb82a19](https://github.com/ophidiarium/cribo/commit/fb82a19dd4432cee7a12f47214b814e5d5047bb0))
* resolve clippy pedantic warnings for pass-by-value arguments ([#252](https://github.com/ophidiarium/cribo/issues/252)) ([4ac881c](https://github.com/ophidiarium/cribo/commit/4ac881ce9b83368eb65d69c3fbb10b0631aa3cec))
* resolve forward reference errors and redundant namespace creation ([#241](https://github.com/ophidiarium/cribo/issues/241)) ([3cef9eb](https://github.com/ophidiarium/cribo/commit/3cef9ebff67ec376b2abd0245adfe72fb8f99600))
* resolve forward reference errors in hard dependency class inheritance ([#232](https://github.com/ophidiarium/cribo/issues/232)) ([b429a7e](https://github.com/ophidiarium/cribo/commit/b429a7e13614cea5338c6f8e8a8ef9ae034084a9))
* resolve metaclass forward references and tree-shaking for same-module dependencies ([#296](https://github.com/ophidiarium/cribo/issues/296)) ([dae1969](https://github.com/ophidiarium/cribo/commit/dae196972535f2ad8082c9b164f26fb47415ee1c))
* resolve metaclass forward references and tree-shaking for same-module dependencies ([#297](https://github.com/ophidiarium/cribo/issues/297)) ([d734046](https://github.com/ophidiarium/cribo/commit/d734046ed6272530d51d3062349b893909c7b96c))
* resolve module import detection for aliased imports ([#57](https://github.com/ophidiarium/cribo/issues/57)) ([0fde5af](https://github.com/ophidiarium/cribo/commit/0fde5af8246b3d59cb1fa9eeabe4c4be1090c25e))
* serpen leftovers ([cfc16bf](https://github.com/ophidiarium/cribo/commit/cfc16bf2be103106db98f37cca39ada96dba3b1f))
* serpen leftovers ([7b9b2a0](https://github.com/ophidiarium/cribo/commit/7b9b2a0c7ffecbff2813bb1712270f7b4ef80358))
* skip self-referential re-export assignments in bundler ([20d8a62](https://github.com/ophidiarium/cribo/commit/20d8a625c696a945bd3d09b84227a9e747b6375e))
* **test:** enforce correct fixture naming for Python execution failures ([#139](https://github.com/ophidiarium/cribo/issues/139)) ([b48923e](https://github.com/ophidiarium/cribo/commit/b48923e9583eb918d57bcb4511d6c8e5609953e0))
* track all dependencies in side-effect modules during tree-shaking ([#288](https://github.com/ophidiarium/cribo/issues/288)) ([c1ef6d2](https://github.com/ophidiarium/cribo/commit/c1ef6d204e9859e3d4d56228571e8ac2c8951e01))
* **tree-shaking:** preserve entry module classes and fix namespace duplication ([#186](https://github.com/ophidiarium/cribo/issues/186)) ([a97c2b6](https://github.com/ophidiarium/cribo/commit/a97c2b6f7e50019fe5920c82182b82e2e0a15068))
* update namespace creation detection for stdlib proxy ([#336](https://github.com/ophidiarium/cribo/issues/336)) ([56bdbb0](https://github.com/ophidiarium/cribo/commit/56bdbb0520d361a322c287aa983a86d8e3a140e4))
* use case-insensitive file extension comparison in util.rs ([6525c4d](https://github.com/ophidiarium/cribo/commit/6525c4d7c79a709257d42c67df4ccdd26d4a514f))
* use curl to call OpenAI API ([b83cb4f](https://github.com/ophidiarium/cribo/commit/b83cb4f40f78f642a1fc6e8f3f1cb20ea61b9caf))
* use original name and declare global ([#221](https://github.com/ophidiarium/cribo/issues/221)) ([04d549b](https://github.com/ophidiarium/cribo/commit/04d549b5e0101412845160795aa0fbe45da5e8d3))
* windows ci ([#39](https://github.com/ophidiarium/cribo/issues/39)) ([8391785](https://github.com/ophidiarium/cribo/commit/8391785bba14f0648322fd00bfdba21106da9ce1))


### Performance Improvements

* **test:** fix slow cli_stdout tests by using pre-built binary ([#149](https://github.com/ophidiarium/cribo/issues/149)) ([e7de937](https://github.com/ophidiarium/cribo/commit/e7de93796cde0cc9f20dce46751323244d71de42))


### Miscellaneous Chores

* release 0.5.0 ([5047e64](https://github.com/ophidiarium/cribo/commit/5047e64bd2546fa8b279b09b49a730db1b81ebac))
* release 0.6.0 ([54c8155](https://github.com/ophidiarium/cribo/commit/54c815570fd2e6d3a14bd182a85b795546ded5a0))
* release 0.7.0 ([5c38833](https://github.com/ophidiarium/cribo/commit/5c388339daf5e5860841faa57463c03aaeb509ca))
* release 0.8.0 ([dc3ef59](https://github.com/ophidiarium/cribo/commit/dc3ef596cdac36c02c3e8f1aa00dade3aa5c0e6b))

## [0.7.0](https://github.com/ophidiarium/cribo/compare/v0.7.0...v0.7.0) (2025-10-24)


### Features

* add __qualname__ ([84bce8d](https://github.com/ophidiarium/cribo/commit/84bce8d9b6f61518bd84a370e7891f77604c34e6))
* add bun and dprint installation to copilot setup workflow ([#287](https://github.com/ophidiarium/cribo/issues/287)) ([751c19c](https://github.com/ophidiarium/cribo/commit/751c19cc329527401373a8ad6c52187fea89f6da))
* add Claude Code hook to prevent direct commits to main branch ([8df0a68](https://github.com/ophidiarium/cribo/commit/8df0a6811625b197029ea5fe8575120568cd855c))
* add comprehensive `from __future__` imports support with generic snapshot testing framework ([#63](https://github.com/ophidiarium/cribo/issues/63)) ([5612dd9](https://github.com/ophidiarium/cribo/commit/5612dd9bab5092d0f73319c8aa7a8bebda7b514b))
* add httpx to ecosystem tests and type_checking_imports fixture ([#306](https://github.com/ophidiarium/cribo/issues/306)) ([dc382e7](https://github.com/ophidiarium/cribo/commit/dc382e7d6f1d273979b2561013cb09e6e047ff8b))
* add idna to ecosystem tests with improved test infrastructure ([#372](https://github.com/ophidiarium/cribo/issues/372)) ([6344432](https://github.com/ophidiarium/cribo/commit/6344432f131e40c2ee1e884cf9d88bd4246d5fd4))
* add VIRTUAL_ENV support ([#40](https://github.com/ophidiarium/cribo/issues/40)) ([70ace41](https://github.com/ophidiarium/cribo/commit/70ace419b1a872c0eb3dd7970206f1e19e73c60b))
* adding aarch64 for linux ([21d3dfa](https://github.com/ophidiarium/cribo/commit/21d3dfac58af6fa12562ce41bcaa395d5675b964))
* **ai:** add AI powered release not summary ([eaf4b2b](https://github.com/ophidiarium/cribo/commit/eaf4b2b27bdc5ac992da401a70b803a475821f4a))
* AST rewrite ([#42](https://github.com/ophidiarium/cribo/issues/42)) ([1172b20](https://github.com/ophidiarium/cribo/commit/1172b202b7170529977f536a39f88a3d91437311))
* **ast:** handle relative imports from parent packages ([#70](https://github.com/ophidiarium/cribo/issues/70)) ([751ac07](https://github.com/ophidiarium/cribo/commit/751ac0708ac2e0991e24ac121ea8b157d956a034))
* **bundler:** ensure sys and types imports follow deterministic ordering ([#113](https://github.com/ophidiarium/cribo/issues/113)) ([05738ee](https://github.com/ophidiarium/cribo/commit/05738ee4aa81de6e424de9aa276db0433c850d23))
* **bundler:** implement static bundling to eliminate runtime exec() calls ([#104](https://github.com/ophidiarium/cribo/issues/104)) ([fceb34f](https://github.com/ophidiarium/cribo/commit/fceb34f3af1c6ef7fc63aee446c7e6481248f003))
* **bundler:** integrate unused import trimming into static bundler ([#108](https://github.com/ophidiarium/cribo/issues/108)) ([d745a4f](https://github.com/ophidiarium/cribo/commit/d745a4ff7ab609f74ad96c7e878e7db41bd1c687))
* **bundler:** migrate unused imports trimmer to graph-based approach ([#115](https://github.com/ophidiarium/cribo/issues/115)) ([5e94496](https://github.com/ophidiarium/cribo/commit/5e9449642c5a2aa5dd12d687d4842bfb2e7a4c1f))
* **bundler:** semantically aware bundler ([#118](https://github.com/ophidiarium/cribo/issues/118)) ([d0be9f8](https://github.com/ophidiarium/cribo/commit/d0be9f8c7883b7369589cd422a5408c47e7a69ee))
* centralize __init__ and __main__ handling via python module ([#349](https://github.com/ophidiarium/cribo/issues/349)) ([9c3a79c](https://github.com/ophidiarium/cribo/commit/9c3a79c43d2c13a1ba7573fbccedb3ccc92be29f))
* **ci:** add comprehensive benchmarking infrastructure ([#75](https://github.com/ophidiarium/cribo/issues/75)) ([ac7a92e](https://github.com/ophidiarium/cribo/commit/ac7a92e001db908c2a000de50f6b65a485886e9e))
* **ci:** add manual trigger support to release-please workflow ([162e501](https://github.com/ophidiarium/cribo/commit/162e50187d62f19a90e8956138488b8e8f0b1683))
* **ci:** add rust-code-analysis-cli ([43523e1](https://github.com/ophidiarium/cribo/commit/43523e1c617949a66f73d63e42bf73c6ce1eb2fe))
* **ci:** only show rust analyzer for changed files ([#132](https://github.com/ophidiarium/cribo/issues/132)) ([de7a875](https://github.com/ophidiarium/cribo/commit/de7a875944acfe9a20219a2da020a8cace7dfd2a))
* **cli:** add stdout output mode for debugging workflows ([#87](https://github.com/ophidiarium/cribo/issues/87)) ([f020c46](https://github.com/ophidiarium/cribo/commit/f020c46eed0bf208056ebf09d242feecb895c975))
* **cli:** add verbose flag repetition support for progressive debugging ([#85](https://github.com/ophidiarium/cribo/issues/85)) ([2fb6941](https://github.com/ophidiarium/cribo/commit/2fb6941b54854e8bdaad51050eaf729adcbbb4f6))
* **docs:** implement dual licensing for documentation ([#54](https://github.com/ophidiarium/cribo/issues/54)) ([24ad056](https://github.com/ophidiarium/cribo/commit/24ad056e086ec5947dc0e0d34faf2031e4e9d150))
* ecosystem tests foundation ([#163](https://github.com/ophidiarium/cribo/issues/163)) ([6695093](https://github.com/ophidiarium/cribo/commit/6695093c80da34d76d2227085bbf33920213d1d3))
* enhance circular dependency detection and prepare for import rewriting ([#126](https://github.com/ophidiarium/cribo/issues/126)) ([6a84d5f](https://github.com/ophidiarium/cribo/commit/6a84d5fa3bbbeba465bd25a59be37505f5d4bb18))
* implement AST visitor pattern for comprehensive import discovery ([#130](https://github.com/ophidiarium/cribo/issues/130)) ([a890b86](https://github.com/ophidiarium/cribo/commit/a890b864342ab421004a2aa3c95da97e2a389406))
* implement automated release management with conventional commits ([#47](https://github.com/ophidiarium/cribo/issues/47)) ([bfd7583](https://github.com/ophidiarium/cribo/commit/bfd7583fa607cd62c22c406fd33d75e8cced8cb7))
* implement centralized namespace management system ([#263](https://github.com/ophidiarium/cribo/issues/263)) ([c9b19f0](https://github.com/ophidiarium/cribo/commit/c9b19f0f209abd9646e2438dbfe0bf91306d7539))
* implement static importlib support and file deduplication ([#157](https://github.com/ophidiarium/cribo/issues/157)) ([77b6609](https://github.com/ophidiarium/cribo/commit/77b6609f99b9767cb6bb0202f1bba13bb61df441))
* implement tree-shaking to remove unused code and imports ([#152](https://github.com/ophidiarium/cribo/issues/152)) ([26c2933](https://github.com/ophidiarium/cribo/commit/26c2933e291f640091fb53cfd5e1e1f8db603ab4))
* implement uv-style hierarchical configuration system ([#51](https://github.com/ophidiarium/cribo/issues/51)) ([bf11be0](https://github.com/ophidiarium/cribo/commit/bf11be0a54d90330f2d645f97cfee28a2eaab4d8))
* integrate Bencher.dev for ecosystem bundling metrics ([#379](https://github.com/ophidiarium/cribo/issues/379)) ([25c293c](https://github.com/ophidiarium/cribo/commit/25c293c14a0cb3e2d387390350215181825f34be))
* integrate ruff linting for bundle output for cross-validation ([#66](https://github.com/ophidiarium/cribo/issues/66)) ([4aff4cc](https://github.com/ophidiarium/cribo/commit/4aff4cc531d731901defb1c6d09d8bf4d8c07045))
* migrate to ruff crates for parsing and AST ([#45](https://github.com/ophidiarium/cribo/issues/45)) ([eab82e3](https://github.com/ophidiarium/cribo/commit/eab82e3f45ff5eaf90a53462f8a264d807408dc9))
* npm publishing workflow ([#35](https://github.com/ophidiarium/cribo/issues/35)) ([cbba549](https://github.com/ophidiarium/cribo/commit/cbba549a8e826707ef3d7dfe59052ab6b561dfba))
* post-checkout hooks ([9063ae2](https://github.com/ophidiarium/cribo/commit/9063ae2b613c1b355440ceb1237d8858fa7390d5))
* **release:** add Aqua and UBI CLI installation support ([#49](https://github.com/ophidiarium/cribo/issues/49)) ([461dc7c](https://github.com/ophidiarium/cribo/commit/461dc7c6a70cbea2611914cba6279bd8d5433534))
* **release:** include npm package.json version management in release-please ([a8fee6f](https://github.com/ophidiarium/cribo/commit/a8fee6faa57bd39448419e560126867d27fc8443))
* smart circular dependency resolution with comprehensive test coverage ([#56](https://github.com/ophidiarium/cribo/issues/56)) ([46f8404](https://github.com/ophidiarium/cribo/commit/46f8404c8f4811bb88a407a53021dc2c92a2d455))
* **test:** enhance snapshot framework with YAML requirements and third-party import support ([#134](https://github.com/ophidiarium/cribo/issues/134)) ([6770e0b](https://github.com/ophidiarium/cribo/commit/6770e0b86ef2c4e06315aabd69ca8bbb2a00a031))
* **test:** re-enable single dot relative import test ([#80](https://github.com/ophidiarium/cribo/issues/80)) ([42d42d8](https://github.com/ophidiarium/cribo/commit/42d42d812c835ea2fef7bb72dfbc820f1aac6590))
* use taplo and stable rust ([8629456](https://github.com/ophidiarium/cribo/commit/86294565102f23b31e1f3f1fcd935c6b5ab02b08))


### Bug Fixes

* add module namespace assignments for wildcard imports in wrapper inits ([#318](https://github.com/ophidiarium/cribo/issues/318)) ([553ef44](https://github.com/ophidiarium/cribo/commit/553ef4400c901192a4de8a8b2d208ec6c52d1544))
* address review comments from PR [#372](https://github.com/ophidiarium/cribo/issues/372) ([#377](https://github.com/ophidiarium/cribo/issues/377)) ([632e9b7](https://github.com/ophidiarium/cribo/commit/632e9b755aed9fe65eb7b4d79a9410b37b408185))
* adjust OpenAI API curl ([26ef069](https://github.com/ophidiarium/cribo/commit/26ef069e2ed6aa002a53f17fb1cb44da5d4e84f7))
* adjust OpenAI API curling ([303e5ad](https://github.com/ophidiarium/cribo/commit/303e5adb921ed82798088cd124482c74d1cb2fef))
* **ai:** improve changelog prompt and use cheaper model ([052fb78](https://github.com/ophidiarium/cribo/commit/052fb782f472c48f6837f22217c509eed2858e6e))
* **ai:** remove LSP recommendations ([9ca7676](https://github.com/ophidiarium/cribo/commit/9ca7676bb5e04659256385a7017c3d6c0325a311))
* apply renames to metaclass keyword arguments in class definitions ([#295](https://github.com/ophidiarium/cribo/issues/295)) ([01f2e1a](https://github.com/ophidiarium/cribo/commit/01f2e1a817f164d14898617577d48617dcda41bb))
* assign init function results to modules in sorted initialization ([#222](https://github.com/ophidiarium/cribo/issues/222)) ([0438108](https://github.com/ophidiarium/cribo/commit/043810880722249783e3a0c0f0fa978d9f77f99c))
* attach entry module exports to namespace for package imports ([#366](https://github.com/ophidiarium/cribo/issues/366)) ([917cc4d](https://github.com/ophidiarium/cribo/commit/917cc4d1b5b340ddf50ac683dae31b959a5b0511))
* base branch bench missed feature ([011ffcd](https://github.com/ophidiarium/cribo/commit/011ffcd91407d5ff1bd806ea2aeb944a8f6e6c8b))
* bencher install ([431ae61](https://github.com/ophidiarium/cribo/commit/431ae61511af683fec2e2988168de27cc55d16ae))
* **bundler:** apply symbol renames to class base classes during inheritance ([#188](https://github.com/ophidiarium/cribo/issues/188)) ([fa2adc4](https://github.com/ophidiarium/cribo/commit/fa2adc42d012c983d04a09a9b1dbce4232148978))
* **bundler:** apply symbol renames to class base classes during inheritance ([#189](https://github.com/ophidiarium/cribo/issues/189)) ([909bd09](https://github.com/ophidiarium/cribo/commit/909bd09dfe53389817bf4651822c940760f7e2df))
* **bundler:** ensure future imports are correctly hoisted and late imports handled ([#112](https://github.com/ophidiarium/cribo/issues/112)) ([1e8ac71](https://github.com/ophidiarium/cribo/commit/1e8ac71f10a89d927d2586c9759f132b1ca0a70d))
* **bundler:** handle __version__ export and eliminate duplicate module assignments ([#213](https://github.com/ophidiarium/cribo/issues/213)) ([9c62e03](https://github.com/ophidiarium/cribo/commit/9c62e03a3a0d1c04c2da35b645297b63efdf3d5e))
* **bundler:** handle circular dependencies with module-level attribute access ([924b9f1](https://github.com/ophidiarium/cribo/commit/924b9f1163587b7b63af706bfab063cd34afd327))
* **bundler:** handle circular dependencies with module-level attribute access ([#219](https://github.com/ophidiarium/cribo/issues/219)) ([ffb49c0](https://github.com/ophidiarium/cribo/commit/ffb49c041c1b04d8e6ac92be41820ce1b10c2255))
* **bundler:** handle conditional imports in if/else and try/except blocks ([#184](https://github.com/ophidiarium/cribo/issues/184)) ([de37ad2](https://github.com/ophidiarium/cribo/commit/de37ad22b4648959ccf146bd382de00fdfca0931))
* **bundler:** preserve import aliases and prevent duplication in hoisted imports ([#135](https://github.com/ophidiarium/cribo/issues/135)) ([632658e](https://github.com/ophidiarium/cribo/commit/632658e19375496b8cce47545802b006d1f5a9bd))
* **bundler:** prevent duplicate namespace assignments when processing parent modules ([#216](https://github.com/ophidiarium/cribo/issues/216)) ([3bcf2a4](https://github.com/ophidiarium/cribo/commit/3bcf2a45eb99992d47f531b3b23c7395d54c2e8c))
* **bundler:** prevent transformation of Python builtins to module attributes ([#212](https://github.com/ophidiarium/cribo/issues/212)) ([1a9b7a9](https://github.com/ophidiarium/cribo/commit/1a9b7a9592a3a5016db30847eb85246c79207c71))
* **bundler:** re-enable package init test and fix parent package imports ([#83](https://github.com/ophidiarium/cribo/issues/83)) ([b352fa4](https://github.com/ophidiarium/cribo/commit/b352fa45a6c4aada2c831b60b03d832f4b6129f9))
* **bundler:** resolve all fixable xfail import test cases ([#120](https://github.com/ophidiarium/cribo/issues/120)) ([bad94a2](https://github.com/ophidiarium/cribo/commit/bad94a230bbd88db10535dade9e483b6ff9bf1e7))
* **bundler:** resolve forward reference issues in cross-module dependencies ([#197](https://github.com/ophidiarium/cribo/issues/197)) ([41b633d](https://github.com/ophidiarium/cribo/commit/41b633d93ffcc413fa0f0e81d2acefd27dcd1fca))
* **bundler:** resolve Python exec scoping and enable module import detection ([#97](https://github.com/ophidiarium/cribo/issues/97)) ([5dab748](https://github.com/ophidiarium/cribo/commit/5dab7489f63549c9900d183477482b02b80dd3e3))
* **bundler:** skip import assignments for tree-shaken symbols ([#214](https://github.com/ophidiarium/cribo/issues/214)) ([6827e4c](https://github.com/ophidiarium/cribo/commit/6827e4c06b8c78ceed61c0000f42951ad6439000))
* **bundler:** wrap modules in circular deps that access imported attributes ([#218](https://github.com/ophidiarium/cribo/issues/218)) ([3f0b093](https://github.com/ophidiarium/cribo/commit/3f0b0934f4fa5eaf7d8f480e9146a35802e52786))
* centralize namespace management to prevent duplicates and fix special module handling ([#261](https://github.com/ophidiarium/cribo/issues/261)) ([c23e8d2](https://github.com/ophidiarium/cribo/commit/c23e8d219cb4e39c398fd4ada330776841dc8ae8))
* **ci:** add missing permissions and explicit command for release-please ([f12f537](https://github.com/ophidiarium/cribo/commit/f12f537702c16ca86a713e2120dea100eb4e62b7))
* **ci:** add missing TAG reference ([5b5bbf6](https://github.com/ophidiarium/cribo/commit/5b5bbf6fa09b433c6f597c98710006d3887091ac))
* **ci:** avoid double run of lint on PRs ([d43dc2a](https://github.com/ophidiarium/cribo/commit/d43dc2acb7618e3d62d49d924e85c37fdd5cd03c))
* **ci:** establish baseline benchmarks for performance tracking ([#77](https://github.com/ophidiarium/cribo/issues/77)) ([98f1385](https://github.com/ophidiarium/cribo/commit/98f1385f2bef1ee19075a6562a20504c07e86b56))
* **ci:** missing -r for jq ([41472ed](https://github.com/ophidiarium/cribo/commit/41472eda758010e6b385daf92936c599f60e6ca2))
* **ci:** remove invalid command parameter from release-please action ([bd171c9](https://github.com/ophidiarium/cribo/commit/bd171c99dad177653726a14dfc8d326e1985d073))
* **ci:** resolve npm package generation and commitlint config issues ([dd70499](https://github.com/ophidiarium/cribo/commit/dd704995b99c6ee17a90a59e1adfa5778b1e9d98))
* **ci:** restore start-point parameters for proper PR benchmarking ([#79](https://github.com/ophidiarium/cribo/issues/79)) ([52a2a2a](https://github.com/ophidiarium/cribo/commit/52a2a2a69c86695e4b5f6507d1960353c82a1ba2))
* **ci:** serpen leftovers ([6d4d3c5](https://github.com/ophidiarium/cribo/commit/6d4d3c585376467f3e4f4d35dc4659995ff6adb3))
* **ci:** use --quiet for codex ([74e876a](https://github.com/ophidiarium/cribo/commit/74e876ae81371d7210c762ccb489481bc539c4b0))
* **ci:** use PAT token and full git history for release-please ([4203d39](https://github.com/ophidiarium/cribo/commit/4203d391b871992bfbeba8054be8b1ad1505e0a3))
* collect dependencies from nested classes and functions in graph builder ([#272](https://github.com/ophidiarium/cribo/issues/272)) ([b021ae1](https://github.com/ophidiarium/cribo/commit/b021ae194d8b5e9b1467fd593c98a99bd6887cc3))
* copilot setup steps ([3e1ecd0](https://github.com/ophidiarium/cribo/commit/3e1ecd09a907593875246ce05dc66f9b52930ceb))
* correctly reference symbols from wrapper modules in namespace assignments ([#298](https://github.com/ophidiarium/cribo/issues/298)) ([91e76e7](https://github.com/ophidiarium/cribo/commit/91e76e7bdc3266eb17bb4e2f27ef53fbfcd267d0))
* **deps:** upgrade ruff crates from 0.11.12 to 0.11.13 ([#122](https://github.com/ophidiarium/cribo/issues/122)) ([9e6c02d](https://github.com/ophidiarium/cribo/commit/9e6c02dd8be2ee58fc55f0555e27eba7413404d1))
* ecosystem testing testing advances ([#165](https://github.com/ophidiarium/cribo/issues/165)) ([a4db95f](https://github.com/ophidiarium/cribo/commit/a4db95fec312341f9247075e332f5b898cd84698))
* ensure private symbols imported by other modules are exported ([#328](https://github.com/ophidiarium/cribo/issues/328)) ([0f467ea](https://github.com/ophidiarium/cribo/commit/0f467ea700feae19efe4c161c5d62d69e1c596f4))
* ensure tree-shaking preserves imports within used functions and classes ([#330](https://github.com/ophidiarium/cribo/issues/330)) ([085695c](https://github.com/ophidiarium/cribo/commit/085695c67aaa2c5a90b59c6b7251e25068055a7b))
* handle built-in type re-exports correctly in bundled output ([#240](https://github.com/ophidiarium/cribo/issues/240)) ([4ab4e83](https://github.com/ophidiarium/cribo/commit/4ab4e83bda56de7ae5b2741a30a8a2a9a2f4a681))
* handle circular dependencies with __version__ module imports ([#314](https://github.com/ophidiarium/cribo/issues/314)) ([c2512e9](https://github.com/ophidiarium/cribo/commit/c2512e98cb232999be11c5e5edb6ed1f82c3da13))
* handle circular dependencies with stdlib-conflicting module names ([#281](https://github.com/ophidiarium/cribo/issues/281)) ([6ea838e](https://github.com/ophidiarium/cribo/commit/6ea838eb24ae2be2b8be73ecc04abb034d033408))
* handle circular imports from parent __init__ modules ([#362](https://github.com/ophidiarium/cribo/issues/362)) ([a8cc60f](https://github.com/ophidiarium/cribo/commit/a8cc60f3b870ba7fef220a4293aa95a687394c4b))
* handle lifted globals correctly in module transformation ([#325](https://github.com/ophidiarium/cribo/issues/325)) ([ff3ae54](https://github.com/ophidiarium/cribo/commit/ff3ae546f75bf2c6e92de0344cdc80cb7e8b8509))
* handle locals() calls in wrapped modules by static analysis ([#308](https://github.com/ophidiarium/cribo/issues/308)) ([c502713](https://github.com/ophidiarium/cribo/commit/c5027135970a73912bab711e384a81cf391cbda5))
* handle metaclass dependencies in class ordering ([1c67f3e](https://github.com/ophidiarium/cribo/commit/1c67f3ef80a742c24906e8434b03da2def0b1d44))
* handle relative imports in wrapper module init functions ([#356](https://github.com/ophidiarium/cribo/issues/356)) ([4c7dfb4](https://github.com/ophidiarium/cribo/commit/4c7dfb4bf17894b041568525075541e6834ecdfd))
* handle stdlib module name conflicts in bundler ([#279](https://github.com/ophidiarium/cribo/issues/279)) ([c800b32](https://github.com/ophidiarium/cribo/commit/c800b32e8c282361a526b8f79d9fe38492385bd4))
* handle submodules in __all__ exports correctly ([8b14937](https://github.com/ophidiarium/cribo/commit/8b14937d16f8f81766f2b727b2336e398dfba3ce))
* handle submodules in __all__ exports correctly ([#226](https://github.com/ophidiarium/cribo/issues/226)) ([b09bce3](https://github.com/ophidiarium/cribo/commit/b09bce3c9e50a0f8d13e8f1d175880439cf1996b))
* handle wildcard imports correctly for wrapper and inlined modules ([#294](https://github.com/ophidiarium/cribo/issues/294)) ([26d5617](https://github.com/ophidiarium/cribo/commit/26d56173a84ccac7d1f3156feb882da526a2a2da))
* handle wildcard imports from inlined modules that re-export wrapper module symbols ([#311](https://github.com/ophidiarium/cribo/issues/311)) ([940f275](https://github.com/ophidiarium/cribo/commit/940f27585f7d76ce6901acc8ad5a3910968ab04a))
* handle wildcard imports in wrapper modules with setattr pattern ([#310](https://github.com/ophidiarium/cribo/issues/310)) ([4db103a](https://github.com/ophidiarium/cribo/commit/4db103a58d949af1b79426c9d371557761e4505c))
* handle wrapper module imports in function default parameters ([#329](https://github.com/ophidiarium/cribo/issues/329)) ([2b4f1bc](https://github.com/ophidiarium/cribo/commit/2b4f1bc34531a142942b3372868f6a8ec2384dd3))
* implement function-scoped import rewriting for circular dependency resolution ([8cc923f](https://github.com/ophidiarium/cribo/commit/8cc923fd6809df0eb0a1b9a4df35d888c0df7bba)), closes [#128](https://github.com/ophidiarium/cribo/issues/128)
* improve class dependency ordering for metaclass and class body references ([#327](https://github.com/ophidiarium/cribo/issues/327)) ([e062df7](https://github.com/ophidiarium/cribo/commit/e062df7162db5bc76bbee4aba697a8c3984b7082))
* improve class ordering for cross-module inheritance ([#277](https://github.com/ophidiarium/cribo/issues/277)) ([392b42a](https://github.com/ophidiarium/cribo/commit/392b42ae535f8842ecd220c7be5e14da8796a734))
* include all module-scope symbols in namespace to support private imports ([#225](https://github.com/ophidiarium/cribo/issues/225)) ([77b77a5](https://github.com/ophidiarium/cribo/commit/77b77a50478d7b05eb7983311ecf6cc1ff5d3d06))
* include explicitly imported private symbols in circular dependencies ([#312](https://github.com/ophidiarium/cribo/issues/312)) ([94e7913](https://github.com/ophidiarium/cribo/commit/94e7913cbef55330c17688a595bd1161ed544af8))
* initialize wrapper modules for lazy imports in inlined modules ([#289](https://github.com/ophidiarium/cribo/issues/289)) ([2db1459](https://github.com/ophidiarium/cribo/commit/2db1459e17368f576c27001295e759c262c733a4))
* install msbuild on windows ([92ffaac](https://github.com/ophidiarium/cribo/commit/92ffaacd7756113c43528f95224b7bd823f2f4e2))
* major ast-rewriter improvement ([#43](https://github.com/ophidiarium/cribo/issues/43)) ([6a71aba](https://github.com/ophidiarium/cribo/commit/6a71aba0eb1df82d15216f103f7fdf8280ecd14f))
* prefer __init__.py over __main__.py for directory entry points ([#364](https://github.com/ophidiarium/cribo/issues/364)) ([2c1e6a9](https://github.com/ophidiarium/cribo/commit/2c1e6a94a910185eec8d7db8c2c33c0992708116))
* preserve aliased imports accessed via module attributes during tree-shaking ([#301](https://github.com/ophidiarium/cribo/issues/301)) ([30916f4](https://github.com/ophidiarium/cribo/commit/30916f4ff5da0b3ea4fc84c8701340f2b65638a7))
* preserve module docstrings in bundled output ([#386](https://github.com/ophidiarium/cribo/issues/386)) ([248a3f0](https://github.com/ophidiarium/cribo/commit/248a3f0e752e3a05bae9a9f71d99658d75b3d833))
* preserve stdlib imports and fix module initialization order for wrapper modules ([#283](https://github.com/ophidiarium/cribo/issues/283)) ([6201f22](https://github.com/ophidiarium/cribo/commit/6201f22d079097a62ca3fc537dfa4a1b16ea80c1))
* preserve symbols accessed dynamically via locals/globals with __all__ ([#317](https://github.com/ophidiarium/cribo/issues/317)) ([fafc7e4](https://github.com/ophidiarium/cribo/commit/fafc7e4bcb86ab788b30db2031cc20088412deca))
* prevent code generator from referencing tree-shaken symbols ([#305](https://github.com/ophidiarium/cribo/issues/305)) ([b8672c4](https://github.com/ophidiarium/cribo/commit/b8672c4175bf085451487be08b083d622f03ba7b))
* prevent globals() transformation in functions within circular dependency modules ([#368](https://github.com/ophidiarium/cribo/issues/368)) ([0e6baea](https://github.com/ophidiarium/cribo/commit/0e6baeaff3b28ce70aec65aaba043e4ab895a634))
* prevent stdlib module name conflicts in bundled imports ([#275](https://github.com/ophidiarium/cribo/issues/275)) ([59b9266](https://github.com/ophidiarium/cribo/commit/59b92662b908e2035dc7e0bb22305f774e0fe795))
* regenerate lockfile ([b036c9c](https://github.com/ophidiarium/cribo/commit/b036c9cf37f5de751cfaf2788ba4e7304ef42a39))
* relative imports being incorrectly classified as stdlib imports ([#267](https://github.com/ophidiarium/cribo/issues/267)) ([a703f0e](https://github.com/ophidiarium/cribo/commit/a703f0e82060a7b59e6ac0c1947867b1217673dd))
* release, again ([77d047b](https://github.com/ophidiarium/cribo/commit/77d047b3f6e3886ed91f74fb450db2c83804bcf7))
* **release:** configure release-please for Cargo workspace ([37a1f74](https://github.com/ophidiarium/cribo/commit/37a1f749f6f3631f804c04d70fdc7fdeecf251b8))
* **release:** reuse release-please version.txt in release workflow ([cbf84f4](https://github.com/ophidiarium/cribo/commit/cbf84f41d3a5fa9c1457f1a9be38146c323ec771))
* remove hardcoded http.cookiejar handling with generic submodule import solution ([#331](https://github.com/ophidiarium/cribo/issues/331)) ([750086d](https://github.com/ophidiarium/cribo/commit/750086d9e971994d7c451f670e4aee89361ecf0d))
* remove unnecessary statement reordering and self-referential wildcard imports ([9e1c8ae](https://github.com/ophidiarium/cribo/commit/9e1c8aee22802a4732b5e903ebadf9778c36d7e3))
* remove unused code to resolve clippy warnings ([bd2ace6](https://github.com/ophidiarium/cribo/commit/bd2ace603ff2142679c4211bf81b342b8729254e))
* remove wheel tags reordering ([65c81e0](https://github.com/ophidiarium/cribo/commit/65c81e0236144e7e358b9c6af096c79f487599f7))
* remove win32-ia32 ([d6dab74](https://github.com/ophidiarium/cribo/commit/d6dab7488703c4a855b36b23d9a8c574a02e14a5))
* rename bundled_exit_code to python_exit_code for clarity ([72f3846](https://github.com/ophidiarium/cribo/commit/72f38463fc00d932b32389f02558ab8a67375ec6))
* replace cast with try_from for leading_dots conversion ([cea9082](https://github.com/ophidiarium/cribo/commit/cea9082c047e970d7e19f9c4e4a93c0d6b943569))
* replace unnecessary Debug formatting with Display for paths ([#260](https://github.com/ophidiarium/cribo/issues/260)) ([e4c6357](https://github.com/ophidiarium/cribo/commit/e4c6357e412c69c115740c7f6f3764e789cd9cdc))
* replacing broken inline python script ([e7854aa](https://github.com/ophidiarium/cribo/commit/e7854aa9d422d564fcde18de586099a66c5fa33f))
* resolve __all__ completely statically ([#247](https://github.com/ophidiarium/cribo/issues/247)) ([fb82a19](https://github.com/ophidiarium/cribo/commit/fb82a19dd4432cee7a12f47214b814e5d5047bb0))
* resolve clippy pedantic warnings for pass-by-value arguments ([#252](https://github.com/ophidiarium/cribo/issues/252)) ([4ac881c](https://github.com/ophidiarium/cribo/commit/4ac881ce9b83368eb65d69c3fbb10b0631aa3cec))
* resolve forward reference errors and redundant namespace creation ([#241](https://github.com/ophidiarium/cribo/issues/241)) ([3cef9eb](https://github.com/ophidiarium/cribo/commit/3cef9ebff67ec376b2abd0245adfe72fb8f99600))
* resolve forward reference errors in hard dependency class inheritance ([#232](https://github.com/ophidiarium/cribo/issues/232)) ([b429a7e](https://github.com/ophidiarium/cribo/commit/b429a7e13614cea5338c6f8e8a8ef9ae034084a9))
* resolve metaclass forward references and tree-shaking for same-module dependencies ([#296](https://github.com/ophidiarium/cribo/issues/296)) ([dae1969](https://github.com/ophidiarium/cribo/commit/dae196972535f2ad8082c9b164f26fb47415ee1c))
* resolve metaclass forward references and tree-shaking for same-module dependencies ([#297](https://github.com/ophidiarium/cribo/issues/297)) ([d734046](https://github.com/ophidiarium/cribo/commit/d734046ed6272530d51d3062349b893909c7b96c))
* resolve module import detection for aliased imports ([#57](https://github.com/ophidiarium/cribo/issues/57)) ([0fde5af](https://github.com/ophidiarium/cribo/commit/0fde5af8246b3d59cb1fa9eeabe4c4be1090c25e))
* serpen leftovers ([cfc16bf](https://github.com/ophidiarium/cribo/commit/cfc16bf2be103106db98f37cca39ada96dba3b1f))
* serpen leftovers ([7b9b2a0](https://github.com/ophidiarium/cribo/commit/7b9b2a0c7ffecbff2813bb1712270f7b4ef80358))
* set version to dynamic ([85b0aad](https://github.com/ophidiarium/cribo/commit/85b0aad615f2be40378cb3019d2f82a6824581a4))
* skip self-referential re-export assignments in bundler ([20d8a62](https://github.com/ophidiarium/cribo/commit/20d8a625c696a945bd3d09b84227a9e747b6375e))
* **test:** enforce correct fixture naming for Python execution failures ([#139](https://github.com/ophidiarium/cribo/issues/139)) ([b48923e](https://github.com/ophidiarium/cribo/commit/b48923e9583eb918d57bcb4511d6c8e5609953e0))
* track all dependencies in side-effect modules during tree-shaking ([#288](https://github.com/ophidiarium/cribo/issues/288)) ([c1ef6d2](https://github.com/ophidiarium/cribo/commit/c1ef6d204e9859e3d4d56228571e8ac2c8951e01))
* **tree-shaking:** preserve entry module classes and fix namespace duplication ([#186](https://github.com/ophidiarium/cribo/issues/186)) ([a97c2b6](https://github.com/ophidiarium/cribo/commit/a97c2b6f7e50019fe5920c82182b82e2e0a15068))
* update namespace creation detection for stdlib proxy ([#336](https://github.com/ophidiarium/cribo/issues/336)) ([56bdbb0](https://github.com/ophidiarium/cribo/commit/56bdbb0520d361a322c287aa983a86d8e3a140e4))
* use case-insensitive file extension comparison in util.rs ([6525c4d](https://github.com/ophidiarium/cribo/commit/6525c4d7c79a709257d42c67df4ccdd26d4a514f))
* use curl to call OpenAI API ([b83cb4f](https://github.com/ophidiarium/cribo/commit/b83cb4f40f78f642a1fc6e8f3f1cb20ea61b9caf))
* use original name and declare global ([#221](https://github.com/ophidiarium/cribo/issues/221)) ([04d549b](https://github.com/ophidiarium/cribo/commit/04d549b5e0101412845160795aa0fbe45da5e8d3))
* use unzip ([8ab5435](https://github.com/ophidiarium/cribo/commit/8ab5435de2a75e0f9347084b6b1373cd8e7d2631))
* windows ci ([#39](https://github.com/ophidiarium/cribo/issues/39)) ([8391785](https://github.com/ophidiarium/cribo/commit/8391785bba14f0648322fd00bfdba21106da9ce1))


### Performance Improvements

* **test:** fix slow cli_stdout tests by using pre-built binary ([#149](https://github.com/ophidiarium/cribo/issues/149)) ([e7de937](https://github.com/ophidiarium/cribo/commit/e7de93796cde0cc9f20dce46751323244d71de42))


### Miscellaneous Chores

* release 0.5.0 ([5047e64](https://github.com/ophidiarium/cribo/commit/5047e64bd2546fa8b279b09b49a730db1b81ebac))
* release 0.6.0 ([54c8155](https://github.com/ophidiarium/cribo/commit/54c815570fd2e6d3a14bd182a85b795546ded5a0))
* release 0.7.0 ([5c38833](https://github.com/ophidiarium/cribo/commit/5c388339daf5e5860841faa57463c03aaeb509ca))

## [0.7.0](https://github.com/ophidiarium/cribo/compare/v0.7.2...v0.7.0) (2025-10-23)


### Features

* add __qualname__ ([84bce8d](https://github.com/ophidiarium/cribo/commit/84bce8d9b6f61518bd84a370e7891f77604c34e6))
* add bun and dprint installation to copilot setup workflow ([#287](https://github.com/ophidiarium/cribo/issues/287)) ([751c19c](https://github.com/ophidiarium/cribo/commit/751c19cc329527401373a8ad6c52187fea89f6da))
* add Claude Code hook to prevent direct commits to main branch ([8df0a68](https://github.com/ophidiarium/cribo/commit/8df0a6811625b197029ea5fe8575120568cd855c))
* add comprehensive `from __future__` imports support with generic snapshot testing framework ([#63](https://github.com/ophidiarium/cribo/issues/63)) ([5612dd9](https://github.com/ophidiarium/cribo/commit/5612dd9bab5092d0f73319c8aa7a8bebda7b514b))
* add httpx to ecosystem tests and type_checking_imports fixture ([#306](https://github.com/ophidiarium/cribo/issues/306)) ([dc382e7](https://github.com/ophidiarium/cribo/commit/dc382e7d6f1d273979b2561013cb09e6e047ff8b))
* add idna to ecosystem tests with improved test infrastructure ([#372](https://github.com/ophidiarium/cribo/issues/372)) ([6344432](https://github.com/ophidiarium/cribo/commit/6344432f131e40c2ee1e884cf9d88bd4246d5fd4))
* add VIRTUAL_ENV support ([#40](https://github.com/ophidiarium/cribo/issues/40)) ([70ace41](https://github.com/ophidiarium/cribo/commit/70ace419b1a872c0eb3dd7970206f1e19e73c60b))
* adding aarch64 for linux ([21d3dfa](https://github.com/ophidiarium/cribo/commit/21d3dfac58af6fa12562ce41bcaa395d5675b964))
* **ai:** add AI powered release not summary ([eaf4b2b](https://github.com/ophidiarium/cribo/commit/eaf4b2b27bdc5ac992da401a70b803a475821f4a))
* AST rewrite ([#42](https://github.com/ophidiarium/cribo/issues/42)) ([1172b20](https://github.com/ophidiarium/cribo/commit/1172b202b7170529977f536a39f88a3d91437311))
* **ast:** handle relative imports from parent packages ([#70](https://github.com/ophidiarium/cribo/issues/70)) ([751ac07](https://github.com/ophidiarium/cribo/commit/751ac0708ac2e0991e24ac121ea8b157d956a034))
* **bundler:** ensure sys and types imports follow deterministic ordering ([#113](https://github.com/ophidiarium/cribo/issues/113)) ([05738ee](https://github.com/ophidiarium/cribo/commit/05738ee4aa81de6e424de9aa276db0433c850d23))
* **bundler:** implement static bundling to eliminate runtime exec() calls ([#104](https://github.com/ophidiarium/cribo/issues/104)) ([fceb34f](https://github.com/ophidiarium/cribo/commit/fceb34f3af1c6ef7fc63aee446c7e6481248f003))
* **bundler:** integrate unused import trimming into static bundler ([#108](https://github.com/ophidiarium/cribo/issues/108)) ([d745a4f](https://github.com/ophidiarium/cribo/commit/d745a4ff7ab609f74ad96c7e878e7db41bd1c687))
* **bundler:** migrate unused imports trimmer to graph-based approach ([#115](https://github.com/ophidiarium/cribo/issues/115)) ([5e94496](https://github.com/ophidiarium/cribo/commit/5e9449642c5a2aa5dd12d687d4842bfb2e7a4c1f))
* **bundler:** semantically aware bundler ([#118](https://github.com/ophidiarium/cribo/issues/118)) ([d0be9f8](https://github.com/ophidiarium/cribo/commit/d0be9f8c7883b7369589cd422a5408c47e7a69ee))
* centralize __init__ and __main__ handling via python module ([#349](https://github.com/ophidiarium/cribo/issues/349)) ([9c3a79c](https://github.com/ophidiarium/cribo/commit/9c3a79c43d2c13a1ba7573fbccedb3ccc92be29f))
* **ci:** add comprehensive benchmarking infrastructure ([#75](https://github.com/ophidiarium/cribo/issues/75)) ([ac7a92e](https://github.com/ophidiarium/cribo/commit/ac7a92e001db908c2a000de50f6b65a485886e9e))
* **ci:** add manual trigger support to release-please workflow ([162e501](https://github.com/ophidiarium/cribo/commit/162e50187d62f19a90e8956138488b8e8f0b1683))
* **ci:** add rust-code-analysis-cli ([43523e1](https://github.com/ophidiarium/cribo/commit/43523e1c617949a66f73d63e42bf73c6ce1eb2fe))
* **ci:** only show rust analyzer for changed files ([#132](https://github.com/ophidiarium/cribo/issues/132)) ([de7a875](https://github.com/ophidiarium/cribo/commit/de7a875944acfe9a20219a2da020a8cace7dfd2a))
* **cli:** add stdout output mode for debugging workflows ([#87](https://github.com/ophidiarium/cribo/issues/87)) ([f020c46](https://github.com/ophidiarium/cribo/commit/f020c46eed0bf208056ebf09d242feecb895c975))
* **cli:** add verbose flag repetition support for progressive debugging ([#85](https://github.com/ophidiarium/cribo/issues/85)) ([2fb6941](https://github.com/ophidiarium/cribo/commit/2fb6941b54854e8bdaad51050eaf729adcbbb4f6))
* **docs:** implement dual licensing for documentation ([#54](https://github.com/ophidiarium/cribo/issues/54)) ([24ad056](https://github.com/ophidiarium/cribo/commit/24ad056e086ec5947dc0e0d34faf2031e4e9d150))
* ecosystem tests foundation ([#163](https://github.com/ophidiarium/cribo/issues/163)) ([6695093](https://github.com/ophidiarium/cribo/commit/6695093c80da34d76d2227085bbf33920213d1d3))
* enhance circular dependency detection and prepare for import rewriting ([#126](https://github.com/ophidiarium/cribo/issues/126)) ([6a84d5f](https://github.com/ophidiarium/cribo/commit/6a84d5fa3bbbeba465bd25a59be37505f5d4bb18))
* implement AST visitor pattern for comprehensive import discovery ([#130](https://github.com/ophidiarium/cribo/issues/130)) ([a890b86](https://github.com/ophidiarium/cribo/commit/a890b864342ab421004a2aa3c95da97e2a389406))
* implement automated release management with conventional commits ([#47](https://github.com/ophidiarium/cribo/issues/47)) ([bfd7583](https://github.com/ophidiarium/cribo/commit/bfd7583fa607cd62c22c406fd33d75e8cced8cb7))
* implement centralized namespace management system ([#263](https://github.com/ophidiarium/cribo/issues/263)) ([c9b19f0](https://github.com/ophidiarium/cribo/commit/c9b19f0f209abd9646e2438dbfe0bf91306d7539))
* implement static importlib support and file deduplication ([#157](https://github.com/ophidiarium/cribo/issues/157)) ([77b6609](https://github.com/ophidiarium/cribo/commit/77b6609f99b9767cb6bb0202f1bba13bb61df441))
* implement tree-shaking to remove unused code and imports ([#152](https://github.com/ophidiarium/cribo/issues/152)) ([26c2933](https://github.com/ophidiarium/cribo/commit/26c2933e291f640091fb53cfd5e1e1f8db603ab4))
* implement uv-style hierarchical configuration system ([#51](https://github.com/ophidiarium/cribo/issues/51)) ([bf11be0](https://github.com/ophidiarium/cribo/commit/bf11be0a54d90330f2d645f97cfee28a2eaab4d8))
* integrate Bencher.dev for ecosystem bundling metrics ([#379](https://github.com/ophidiarium/cribo/issues/379)) ([25c293c](https://github.com/ophidiarium/cribo/commit/25c293c14a0cb3e2d387390350215181825f34be))
* integrate ruff linting for bundle output for cross-validation ([#66](https://github.com/ophidiarium/cribo/issues/66)) ([4aff4cc](https://github.com/ophidiarium/cribo/commit/4aff4cc531d731901defb1c6d09d8bf4d8c07045))
* migrate to ruff crates for parsing and AST ([#45](https://github.com/ophidiarium/cribo/issues/45)) ([eab82e3](https://github.com/ophidiarium/cribo/commit/eab82e3f45ff5eaf90a53462f8a264d807408dc9))
* npm publishing workflow ([#35](https://github.com/ophidiarium/cribo/issues/35)) ([cbba549](https://github.com/ophidiarium/cribo/commit/cbba549a8e826707ef3d7dfe59052ab6b561dfba))
* post-checkout hooks ([9063ae2](https://github.com/ophidiarium/cribo/commit/9063ae2b613c1b355440ceb1237d8858fa7390d5))
* **release:** add Aqua and UBI CLI installation support ([#49](https://github.com/ophidiarium/cribo/issues/49)) ([461dc7c](https://github.com/ophidiarium/cribo/commit/461dc7c6a70cbea2611914cba6279bd8d5433534))
* **release:** include npm package.json version management in release-please ([a8fee6f](https://github.com/ophidiarium/cribo/commit/a8fee6faa57bd39448419e560126867d27fc8443))
* smart circular dependency resolution with comprehensive test coverage ([#56](https://github.com/ophidiarium/cribo/issues/56)) ([46f8404](https://github.com/ophidiarium/cribo/commit/46f8404c8f4811bb88a407a53021dc2c92a2d455))
* **test:** enhance snapshot framework with YAML requirements and third-party import support ([#134](https://github.com/ophidiarium/cribo/issues/134)) ([6770e0b](https://github.com/ophidiarium/cribo/commit/6770e0b86ef2c4e06315aabd69ca8bbb2a00a031))
* **test:** re-enable single dot relative import test ([#80](https://github.com/ophidiarium/cribo/issues/80)) ([42d42d8](https://github.com/ophidiarium/cribo/commit/42d42d812c835ea2fef7bb72dfbc820f1aac6590))
* use taplo and stable rust ([8629456](https://github.com/ophidiarium/cribo/commit/86294565102f23b31e1f3f1fcd935c6b5ab02b08))


### Bug Fixes

* add ignore-nothing-to-cache: true ([2bb4c5e](https://github.com/ophidiarium/cribo/commit/2bb4c5ee3aa5d71c92efb917e469003ab59afe60))
* add module namespace assignments for wildcard imports in wrapper inits ([#318](https://github.com/ophidiarium/cribo/issues/318)) ([553ef44](https://github.com/ophidiarium/cribo/commit/553ef4400c901192a4de8a8b2d208ec6c52d1544))
* address review comments from PR [#372](https://github.com/ophidiarium/cribo/issues/372) ([#377](https://github.com/ophidiarium/cribo/issues/377)) ([632e9b7](https://github.com/ophidiarium/cribo/commit/632e9b755aed9fe65eb7b4d79a9410b37b408185))
* adjust OpenAI API curl ([26ef069](https://github.com/ophidiarium/cribo/commit/26ef069e2ed6aa002a53f17fb1cb44da5d4e84f7))
* adjust OpenAI API curling ([303e5ad](https://github.com/ophidiarium/cribo/commit/303e5adb921ed82798088cd124482c74d1cb2fef))
* **ai:** improve changelog prompt and use cheaper model ([052fb78](https://github.com/ophidiarium/cribo/commit/052fb782f472c48f6837f22217c509eed2858e6e))
* **ai:** remove LSP recommendations ([9ca7676](https://github.com/ophidiarium/cribo/commit/9ca7676bb5e04659256385a7017c3d6c0325a311))
* apply renames to metaclass keyword arguments in class definitions ([#295](https://github.com/ophidiarium/cribo/issues/295)) ([01f2e1a](https://github.com/ophidiarium/cribo/commit/01f2e1a817f164d14898617577d48617dcda41bb))
* assign init function results to modules in sorted initialization ([#222](https://github.com/ophidiarium/cribo/issues/222)) ([0438108](https://github.com/ophidiarium/cribo/commit/043810880722249783e3a0c0f0fa978d9f77f99c))
* attach entry module exports to namespace for package imports ([#366](https://github.com/ophidiarium/cribo/issues/366)) ([917cc4d](https://github.com/ophidiarium/cribo/commit/917cc4d1b5b340ddf50ac683dae31b959a5b0511))
* base branch bench missed feature ([011ffcd](https://github.com/ophidiarium/cribo/commit/011ffcd91407d5ff1bd806ea2aeb944a8f6e6c8b))
* bencher install ([431ae61](https://github.com/ophidiarium/cribo/commit/431ae61511af683fec2e2988168de27cc55d16ae))
* **bundler:** apply symbol renames to class base classes during inheritance ([#188](https://github.com/ophidiarium/cribo/issues/188)) ([fa2adc4](https://github.com/ophidiarium/cribo/commit/fa2adc42d012c983d04a09a9b1dbce4232148978))
* **bundler:** apply symbol renames to class base classes during inheritance ([#189](https://github.com/ophidiarium/cribo/issues/189)) ([909bd09](https://github.com/ophidiarium/cribo/commit/909bd09dfe53389817bf4651822c940760f7e2df))
* **bundler:** ensure future imports are correctly hoisted and late imports handled ([#112](https://github.com/ophidiarium/cribo/issues/112)) ([1e8ac71](https://github.com/ophidiarium/cribo/commit/1e8ac71f10a89d927d2586c9759f132b1ca0a70d))
* **bundler:** handle __version__ export and eliminate duplicate module assignments ([#213](https://github.com/ophidiarium/cribo/issues/213)) ([9c62e03](https://github.com/ophidiarium/cribo/commit/9c62e03a3a0d1c04c2da35b645297b63efdf3d5e))
* **bundler:** handle circular dependencies with module-level attribute access ([924b9f1](https://github.com/ophidiarium/cribo/commit/924b9f1163587b7b63af706bfab063cd34afd327))
* **bundler:** handle circular dependencies with module-level attribute access ([#219](https://github.com/ophidiarium/cribo/issues/219)) ([ffb49c0](https://github.com/ophidiarium/cribo/commit/ffb49c041c1b04d8e6ac92be41820ce1b10c2255))
* **bundler:** handle conditional imports in if/else and try/except blocks ([#184](https://github.com/ophidiarium/cribo/issues/184)) ([de37ad2](https://github.com/ophidiarium/cribo/commit/de37ad22b4648959ccf146bd382de00fdfca0931))
* **bundler:** preserve import aliases and prevent duplication in hoisted imports ([#135](https://github.com/ophidiarium/cribo/issues/135)) ([632658e](https://github.com/ophidiarium/cribo/commit/632658e19375496b8cce47545802b006d1f5a9bd))
* **bundler:** prevent duplicate namespace assignments when processing parent modules ([#216](https://github.com/ophidiarium/cribo/issues/216)) ([3bcf2a4](https://github.com/ophidiarium/cribo/commit/3bcf2a45eb99992d47f531b3b23c7395d54c2e8c))
* **bundler:** prevent transformation of Python builtins to module attributes ([#212](https://github.com/ophidiarium/cribo/issues/212)) ([1a9b7a9](https://github.com/ophidiarium/cribo/commit/1a9b7a9592a3a5016db30847eb85246c79207c71))
* **bundler:** re-enable package init test and fix parent package imports ([#83](https://github.com/ophidiarium/cribo/issues/83)) ([b352fa4](https://github.com/ophidiarium/cribo/commit/b352fa45a6c4aada2c831b60b03d832f4b6129f9))
* **bundler:** resolve all fixable xfail import test cases ([#120](https://github.com/ophidiarium/cribo/issues/120)) ([bad94a2](https://github.com/ophidiarium/cribo/commit/bad94a230bbd88db10535dade9e483b6ff9bf1e7))
* **bundler:** resolve forward reference issues in cross-module dependencies ([#197](https://github.com/ophidiarium/cribo/issues/197)) ([41b633d](https://github.com/ophidiarium/cribo/commit/41b633d93ffcc413fa0f0e81d2acefd27dcd1fca))
* **bundler:** resolve Python exec scoping and enable module import detection ([#97](https://github.com/ophidiarium/cribo/issues/97)) ([5dab748](https://github.com/ophidiarium/cribo/commit/5dab7489f63549c9900d183477482b02b80dd3e3))
* **bundler:** skip import assignments for tree-shaken symbols ([#214](https://github.com/ophidiarium/cribo/issues/214)) ([6827e4c](https://github.com/ophidiarium/cribo/commit/6827e4c06b8c78ceed61c0000f42951ad6439000))
* **bundler:** wrap modules in circular deps that access imported attributes ([#218](https://github.com/ophidiarium/cribo/issues/218)) ([3f0b093](https://github.com/ophidiarium/cribo/commit/3f0b0934f4fa5eaf7d8f480e9146a35802e52786))
* centralize namespace management to prevent duplicates and fix special module handling ([#261](https://github.com/ophidiarium/cribo/issues/261)) ([c23e8d2](https://github.com/ophidiarium/cribo/commit/c23e8d219cb4e39c398fd4ada330776841dc8ae8))
* **ci:** add missing permissions and explicit command for release-please ([f12f537](https://github.com/ophidiarium/cribo/commit/f12f537702c16ca86a713e2120dea100eb4e62b7))
* **ci:** add missing TAG reference ([5b5bbf6](https://github.com/ophidiarium/cribo/commit/5b5bbf6fa09b433c6f597c98710006d3887091ac))
* **ci:** avoid double run of lint on PRs ([d43dc2a](https://github.com/ophidiarium/cribo/commit/d43dc2acb7618e3d62d49d924e85c37fdd5cd03c))
* **ci:** establish baseline benchmarks for performance tracking ([#77](https://github.com/ophidiarium/cribo/issues/77)) ([98f1385](https://github.com/ophidiarium/cribo/commit/98f1385f2bef1ee19075a6562a20504c07e86b56))
* **ci:** missing -r for jq ([41472ed](https://github.com/ophidiarium/cribo/commit/41472eda758010e6b385daf92936c599f60e6ca2))
* **ci:** remove invalid command parameter from release-please action ([bd171c9](https://github.com/ophidiarium/cribo/commit/bd171c99dad177653726a14dfc8d326e1985d073))
* **ci:** resolve npm package generation and commitlint config issues ([dd70499](https://github.com/ophidiarium/cribo/commit/dd704995b99c6ee17a90a59e1adfa5778b1e9d98))
* **ci:** restore start-point parameters for proper PR benchmarking ([#79](https://github.com/ophidiarium/cribo/issues/79)) ([52a2a2a](https://github.com/ophidiarium/cribo/commit/52a2a2a69c86695e4b5f6507d1960353c82a1ba2))
* **ci:** serpen leftovers ([6d4d3c5](https://github.com/ophidiarium/cribo/commit/6d4d3c585376467f3e4f4d35dc4659995ff6adb3))
* **ci:** use --quiet for codex ([74e876a](https://github.com/ophidiarium/cribo/commit/74e876ae81371d7210c762ccb489481bc539c4b0))
* **ci:** use PAT token and full git history for release-please ([4203d39](https://github.com/ophidiarium/cribo/commit/4203d391b871992bfbeba8054be8b1ad1505e0a3))
* collect dependencies from nested classes and functions in graph builder ([#272](https://github.com/ophidiarium/cribo/issues/272)) ([b021ae1](https://github.com/ophidiarium/cribo/commit/b021ae194d8b5e9b1467fd593c98a99bd6887cc3))
* copilot setup steps ([3e1ecd0](https://github.com/ophidiarium/cribo/commit/3e1ecd09a907593875246ce05dc66f9b52930ceb))
* correctly reference symbols from wrapper modules in namespace assignments ([#298](https://github.com/ophidiarium/cribo/issues/298)) ([91e76e7](https://github.com/ophidiarium/cribo/commit/91e76e7bdc3266eb17bb4e2f27ef53fbfcd267d0))
* **deps:** upgrade ruff crates from 0.11.12 to 0.11.13 ([#122](https://github.com/ophidiarium/cribo/issues/122)) ([9e6c02d](https://github.com/ophidiarium/cribo/commit/9e6c02dd8be2ee58fc55f0555e27eba7413404d1))
* ecosystem testing testing advances ([#165](https://github.com/ophidiarium/cribo/issues/165)) ([a4db95f](https://github.com/ophidiarium/cribo/commit/a4db95fec312341f9247075e332f5b898cd84698))
* ensure private symbols imported by other modules are exported ([#328](https://github.com/ophidiarium/cribo/issues/328)) ([0f467ea](https://github.com/ophidiarium/cribo/commit/0f467ea700feae19efe4c161c5d62d69e1c596f4))
* ensure tree-shaking preserves imports within used functions and classes ([#330](https://github.com/ophidiarium/cribo/issues/330)) ([085695c](https://github.com/ophidiarium/cribo/commit/085695c67aaa2c5a90b59c6b7251e25068055a7b))
* handle built-in type re-exports correctly in bundled output ([#240](https://github.com/ophidiarium/cribo/issues/240)) ([4ab4e83](https://github.com/ophidiarium/cribo/commit/4ab4e83bda56de7ae5b2741a30a8a2a9a2f4a681))
* handle circular dependencies with __version__ module imports ([#314](https://github.com/ophidiarium/cribo/issues/314)) ([c2512e9](https://github.com/ophidiarium/cribo/commit/c2512e98cb232999be11c5e5edb6ed1f82c3da13))
* handle circular dependencies with stdlib-conflicting module names ([#281](https://github.com/ophidiarium/cribo/issues/281)) ([6ea838e](https://github.com/ophidiarium/cribo/commit/6ea838eb24ae2be2b8be73ecc04abb034d033408))
* handle circular imports from parent __init__ modules ([#362](https://github.com/ophidiarium/cribo/issues/362)) ([a8cc60f](https://github.com/ophidiarium/cribo/commit/a8cc60f3b870ba7fef220a4293aa95a687394c4b))
* handle lifted globals correctly in module transformation ([#325](https://github.com/ophidiarium/cribo/issues/325)) ([ff3ae54](https://github.com/ophidiarium/cribo/commit/ff3ae546f75bf2c6e92de0344cdc80cb7e8b8509))
* handle locals() calls in wrapped modules by static analysis ([#308](https://github.com/ophidiarium/cribo/issues/308)) ([c502713](https://github.com/ophidiarium/cribo/commit/c5027135970a73912bab711e384a81cf391cbda5))
* handle metaclass dependencies in class ordering ([1c67f3e](https://github.com/ophidiarium/cribo/commit/1c67f3ef80a742c24906e8434b03da2def0b1d44))
* handle relative imports in wrapper module init functions ([#356](https://github.com/ophidiarium/cribo/issues/356)) ([4c7dfb4](https://github.com/ophidiarium/cribo/commit/4c7dfb4bf17894b041568525075541e6834ecdfd))
* handle stdlib module name conflicts in bundler ([#279](https://github.com/ophidiarium/cribo/issues/279)) ([c800b32](https://github.com/ophidiarium/cribo/commit/c800b32e8c282361a526b8f79d9fe38492385bd4))
* handle submodules in __all__ exports correctly ([8b14937](https://github.com/ophidiarium/cribo/commit/8b14937d16f8f81766f2b727b2336e398dfba3ce))
* handle submodules in __all__ exports correctly ([#226](https://github.com/ophidiarium/cribo/issues/226)) ([b09bce3](https://github.com/ophidiarium/cribo/commit/b09bce3c9e50a0f8d13e8f1d175880439cf1996b))
* handle wildcard imports correctly for wrapper and inlined modules ([#294](https://github.com/ophidiarium/cribo/issues/294)) ([26d5617](https://github.com/ophidiarium/cribo/commit/26d56173a84ccac7d1f3156feb882da526a2a2da))
* handle wildcard imports from inlined modules that re-export wrapper module symbols ([#311](https://github.com/ophidiarium/cribo/issues/311)) ([940f275](https://github.com/ophidiarium/cribo/commit/940f27585f7d76ce6901acc8ad5a3910968ab04a))
* handle wildcard imports in wrapper modules with setattr pattern ([#310](https://github.com/ophidiarium/cribo/issues/310)) ([4db103a](https://github.com/ophidiarium/cribo/commit/4db103a58d949af1b79426c9d371557761e4505c))
* handle wrapper module imports in function default parameters ([#329](https://github.com/ophidiarium/cribo/issues/329)) ([2b4f1bc](https://github.com/ophidiarium/cribo/commit/2b4f1bc34531a142942b3372868f6a8ec2384dd3))
* implement function-scoped import rewriting for circular dependency resolution ([8cc923f](https://github.com/ophidiarium/cribo/commit/8cc923fd6809df0eb0a1b9a4df35d888c0df7bba)), closes [#128](https://github.com/ophidiarium/cribo/issues/128)
* improve class dependency ordering for metaclass and class body references ([#327](https://github.com/ophidiarium/cribo/issues/327)) ([e062df7](https://github.com/ophidiarium/cribo/commit/e062df7162db5bc76bbee4aba697a8c3984b7082))
* improve class ordering for cross-module inheritance ([#277](https://github.com/ophidiarium/cribo/issues/277)) ([392b42a](https://github.com/ophidiarium/cribo/commit/392b42ae535f8842ecd220c7be5e14da8796a734))
* include all module-scope symbols in namespace to support private imports ([#225](https://github.com/ophidiarium/cribo/issues/225)) ([77b77a5](https://github.com/ophidiarium/cribo/commit/77b77a50478d7b05eb7983311ecf6cc1ff5d3d06))
* include explicitly imported private symbols in circular dependencies ([#312](https://github.com/ophidiarium/cribo/issues/312)) ([94e7913](https://github.com/ophidiarium/cribo/commit/94e7913cbef55330c17688a595bd1161ed544af8))
* initialize wrapper modules for lazy imports in inlined modules ([#289](https://github.com/ophidiarium/cribo/issues/289)) ([2db1459](https://github.com/ophidiarium/cribo/commit/2db1459e17368f576c27001295e759c262c733a4))
* install msbuild on windows ([92ffaac](https://github.com/ophidiarium/cribo/commit/92ffaacd7756113c43528f95224b7bd823f2f4e2))
* major ast-rewriter improvement ([#43](https://github.com/ophidiarium/cribo/issues/43)) ([6a71aba](https://github.com/ophidiarium/cribo/commit/6a71aba0eb1df82d15216f103f7fdf8280ecd14f))
* prefer __init__.py over __main__.py for directory entry points ([#364](https://github.com/ophidiarium/cribo/issues/364)) ([2c1e6a9](https://github.com/ophidiarium/cribo/commit/2c1e6a94a910185eec8d7db8c2c33c0992708116))
* preserve aliased imports accessed via module attributes during tree-shaking ([#301](https://github.com/ophidiarium/cribo/issues/301)) ([30916f4](https://github.com/ophidiarium/cribo/commit/30916f4ff5da0b3ea4fc84c8701340f2b65638a7))
* preserve module docstrings in bundled output ([#386](https://github.com/ophidiarium/cribo/issues/386)) ([248a3f0](https://github.com/ophidiarium/cribo/commit/248a3f0e752e3a05bae9a9f71d99658d75b3d833))
* preserve stdlib imports and fix module initialization order for wrapper modules ([#283](https://github.com/ophidiarium/cribo/issues/283)) ([6201f22](https://github.com/ophidiarium/cribo/commit/6201f22d079097a62ca3fc537dfa4a1b16ea80c1))
* preserve symbols accessed dynamically via locals/globals with __all__ ([#317](https://github.com/ophidiarium/cribo/issues/317)) ([fafc7e4](https://github.com/ophidiarium/cribo/commit/fafc7e4bcb86ab788b30db2031cc20088412deca))
* prevent code generator from referencing tree-shaken symbols ([#305](https://github.com/ophidiarium/cribo/issues/305)) ([b8672c4](https://github.com/ophidiarium/cribo/commit/b8672c4175bf085451487be08b083d622f03ba7b))
* prevent globals() transformation in functions within circular dependency modules ([#368](https://github.com/ophidiarium/cribo/issues/368)) ([0e6baea](https://github.com/ophidiarium/cribo/commit/0e6baeaff3b28ce70aec65aaba043e4ab895a634))
* prevent stdlib module name conflicts in bundled imports ([#275](https://github.com/ophidiarium/cribo/issues/275)) ([59b9266](https://github.com/ophidiarium/cribo/commit/59b92662b908e2035dc7e0bb22305f774e0fe795))
* publishing new versions ([b6b905f](https://github.com/ophidiarium/cribo/commit/b6b905fa45c0827fd355f08ff7d0a230df070bed))
* regenerate lockfile ([b036c9c](https://github.com/ophidiarium/cribo/commit/b036c9cf37f5de751cfaf2788ba4e7304ef42a39))
* relative imports being incorrectly classified as stdlib imports ([#267](https://github.com/ophidiarium/cribo/issues/267)) ([a703f0e](https://github.com/ophidiarium/cribo/commit/a703f0e82060a7b59e6ac0c1947867b1217673dd))
* release, again ([77d047b](https://github.com/ophidiarium/cribo/commit/77d047b3f6e3886ed91f74fb450db2c83804bcf7))
* **release:** configure release-please for Cargo workspace ([37a1f74](https://github.com/ophidiarium/cribo/commit/37a1f749f6f3631f804c04d70fdc7fdeecf251b8))
* **release:** reuse release-please version.txt in release workflow ([cbf84f4](https://github.com/ophidiarium/cribo/commit/cbf84f41d3a5fa9c1457f1a9be38146c323ec771))
* remove hardcoded http.cookiejar handling with generic submodule import solution ([#331](https://github.com/ophidiarium/cribo/issues/331)) ([750086d](https://github.com/ophidiarium/cribo/commit/750086d9e971994d7c451f670e4aee89361ecf0d))
* remove unnecessary statement reordering and self-referential wildcard imports ([9e1c8ae](https://github.com/ophidiarium/cribo/commit/9e1c8aee22802a4732b5e903ebadf9778c36d7e3))
* remove unused code to resolve clippy warnings ([bd2ace6](https://github.com/ophidiarium/cribo/commit/bd2ace603ff2142679c4211bf81b342b8729254e))
* remove wheel tags reordering ([65c81e0](https://github.com/ophidiarium/cribo/commit/65c81e0236144e7e358b9c6af096c79f487599f7))
* remove win32-ia32 ([d6dab74](https://github.com/ophidiarium/cribo/commit/d6dab7488703c4a855b36b23d9a8c574a02e14a5))
* rename bundled_exit_code to python_exit_code for clarity ([72f3846](https://github.com/ophidiarium/cribo/commit/72f38463fc00d932b32389f02558ab8a67375ec6))
* replace cast with try_from for leading_dots conversion ([cea9082](https://github.com/ophidiarium/cribo/commit/cea9082c047e970d7e19f9c4e4a93c0d6b943569))
* replace unnecessary Debug formatting with Display for paths ([#260](https://github.com/ophidiarium/cribo/issues/260)) ([e4c6357](https://github.com/ophidiarium/cribo/commit/e4c6357e412c69c115740c7f6f3764e789cd9cdc))
* replacing broken inline python script ([e7854aa](https://github.com/ophidiarium/cribo/commit/e7854aa9d422d564fcde18de586099a66c5fa33f))
* resolve __all__ completely statically ([#247](https://github.com/ophidiarium/cribo/issues/247)) ([fb82a19](https://github.com/ophidiarium/cribo/commit/fb82a19dd4432cee7a12f47214b814e5d5047bb0))
* resolve clippy pedantic warnings for pass-by-value arguments ([#252](https://github.com/ophidiarium/cribo/issues/252)) ([4ac881c](https://github.com/ophidiarium/cribo/commit/4ac881ce9b83368eb65d69c3fbb10b0631aa3cec))
* resolve forward reference errors and redundant namespace creation ([#241](https://github.com/ophidiarium/cribo/issues/241)) ([3cef9eb](https://github.com/ophidiarium/cribo/commit/3cef9ebff67ec376b2abd0245adfe72fb8f99600))
* resolve forward reference errors in hard dependency class inheritance ([#232](https://github.com/ophidiarium/cribo/issues/232)) ([b429a7e](https://github.com/ophidiarium/cribo/commit/b429a7e13614cea5338c6f8e8a8ef9ae034084a9))
* resolve metaclass forward references and tree-shaking for same-module dependencies ([#296](https://github.com/ophidiarium/cribo/issues/296)) ([dae1969](https://github.com/ophidiarium/cribo/commit/dae196972535f2ad8082c9b164f26fb47415ee1c))
* resolve metaclass forward references and tree-shaking for same-module dependencies ([#297](https://github.com/ophidiarium/cribo/issues/297)) ([d734046](https://github.com/ophidiarium/cribo/commit/d734046ed6272530d51d3062349b893909c7b96c))
* resolve module import detection for aliased imports ([#57](https://github.com/ophidiarium/cribo/issues/57)) ([0fde5af](https://github.com/ophidiarium/cribo/commit/0fde5af8246b3d59cb1fa9eeabe4c4be1090c25e))
* serpen leftovers ([cfc16bf](https://github.com/ophidiarium/cribo/commit/cfc16bf2be103106db98f37cca39ada96dba3b1f))
* serpen leftovers ([7b9b2a0](https://github.com/ophidiarium/cribo/commit/7b9b2a0c7ffecbff2813bb1712270f7b4ef80358))
* set version to dynamic ([85b0aad](https://github.com/ophidiarium/cribo/commit/85b0aad615f2be40378cb3019d2f82a6824581a4))
* skip self-referential re-export assignments in bundler ([20d8a62](https://github.com/ophidiarium/cribo/commit/20d8a625c696a945bd3d09b84227a9e747b6375e))
* **test:** enforce correct fixture naming for Python execution failures ([#139](https://github.com/ophidiarium/cribo/issues/139)) ([b48923e](https://github.com/ophidiarium/cribo/commit/b48923e9583eb918d57bcb4511d6c8e5609953e0))
* track all dependencies in side-effect modules during tree-shaking ([#288](https://github.com/ophidiarium/cribo/issues/288)) ([c1ef6d2](https://github.com/ophidiarium/cribo/commit/c1ef6d204e9859e3d4d56228571e8ac2c8951e01))
* **tree-shaking:** preserve entry module classes and fix namespace duplication ([#186](https://github.com/ophidiarium/cribo/issues/186)) ([a97c2b6](https://github.com/ophidiarium/cribo/commit/a97c2b6f7e50019fe5920c82182b82e2e0a15068))
* update namespace creation detection for stdlib proxy ([#336](https://github.com/ophidiarium/cribo/issues/336)) ([56bdbb0](https://github.com/ophidiarium/cribo/commit/56bdbb0520d361a322c287aa983a86d8e3a140e4))
* use case-insensitive file extension comparison in util.rs ([6525c4d](https://github.com/ophidiarium/cribo/commit/6525c4d7c79a709257d42c67df4ccdd26d4a514f))
* use curl to call OpenAI API ([b83cb4f](https://github.com/ophidiarium/cribo/commit/b83cb4f40f78f642a1fc6e8f3f1cb20ea61b9caf))
* use original name and declare global ([#221](https://github.com/ophidiarium/cribo/issues/221)) ([04d549b](https://github.com/ophidiarium/cribo/commit/04d549b5e0101412845160795aa0fbe45da5e8d3))
* use unzip ([8ab5435](https://github.com/ophidiarium/cribo/commit/8ab5435de2a75e0f9347084b6b1373cd8e7d2631))
* windows ci ([#39](https://github.com/ophidiarium/cribo/issues/39)) ([8391785](https://github.com/ophidiarium/cribo/commit/8391785bba14f0648322fd00bfdba21106da9ce1))


### Performance Improvements

* **test:** fix slow cli_stdout tests by using pre-built binary ([#149](https://github.com/ophidiarium/cribo/issues/149)) ([e7de937](https://github.com/ophidiarium/cribo/commit/e7de93796cde0cc9f20dce46751323244d71de42))


### Miscellaneous Chores

* release 0.5.0 ([5047e64](https://github.com/ophidiarium/cribo/commit/5047e64bd2546fa8b279b09b49a730db1b81ebac))
* release 0.6.0 ([54c8155](https://github.com/ophidiarium/cribo/commit/54c815570fd2e6d3a14bd182a85b795546ded5a0))
* release 0.7.0 ([5c38833](https://github.com/ophidiarium/cribo/commit/5c388339daf5e5860841faa57463c03aaeb509ca))

## [0.7.2](https://github.com/ophidiarium/cribo/compare/v0.7.1...v0.7.2) (2025-10-01)


### Features

* integrate Bencher.dev for ecosystem bundling metrics ([#379](https://github.com/ophidiarium/cribo/issues/379)) ([5518554](https://github.com/ophidiarium/cribo/commit/5518554dfeaedbbb7346927f49f109f9abfb27f4))


### Bug Fixes

* address review comments from PR [#372](https://github.com/ophidiarium/cribo/issues/372) ([#377](https://github.com/ophidiarium/cribo/issues/377)) ([931e2ef](https://github.com/ophidiarium/cribo/commit/931e2efca2b97adbcca0ac9140aa9c9d6a9327b6))
* bencher install ([d700482](https://github.com/ophidiarium/cribo/commit/d7004820c7a8494c3febb3c5194aeb91f7d73917))
* regenerate lockfile ([f9c0831](https://github.com/ophidiarium/cribo/commit/f9c083158fe155de2f9aa68201f2c73e45960301))

## [0.7.1](https://github.com/ophidiarium/cribo/compare/v0.7.0...v0.7.1) (2025-09-29)


### Features

* add idna to ecosystem tests with improved test infrastructure ([#372](https://github.com/ophidiarium/cribo/issues/372)) ([fcfcf04](https://github.com/ophidiarium/cribo/commit/fcfcf04e66080a4d02147f0eaf9b25f3fbe363dc))

## [0.7.0](https://github.com/ophidiarium/cribo/compare/v0.6.1...v0.7.0) (2025-09-17)


### Bug Fixes

* attach entry module exports to namespace for package imports ([#366](https://github.com/ophidiarium/cribo/issues/366)) ([c299d4c](https://github.com/ophidiarium/cribo/commit/c299d4ca82f6d5fd2fbce141081fcb399043a1a8))
* handle circular imports from parent __init__ modules ([#362](https://github.com/ophidiarium/cribo/issues/362)) ([ac4348c](https://github.com/ophidiarium/cribo/commit/ac4348c744ffb32635fec913d6f5fcd710c14e70))
* prefer __init__.py over __main__.py for directory entry points ([#364](https://github.com/ophidiarium/cribo/issues/364)) ([6082ce0](https://github.com/ophidiarium/cribo/commit/6082ce0ed9e6b5ca2480369ba53a46a4a3b33931))
* prevent globals() transformation in functions within circular dependency modules ([#368](https://github.com/ophidiarium/cribo/issues/368)) ([5cb639b](https://github.com/ophidiarium/cribo/commit/5cb639beb532d947a92aa05be0772fe7aa6f4389))


### Miscellaneous Chores

* release 0.7.0 ([f043cff](https://github.com/ophidiarium/cribo/commit/f043cffd65ffd65ef4c7cd4d2e37bda20b16488b))

## [0.6.1](https://github.com/ophidiarium/cribo/compare/v0.6.0...v0.6.1) (2025-09-13)


### Bug Fixes

* handle relative imports in wrapper module init functions ([#356](https://github.com/ophidiarium/cribo/issues/356)) ([3aafafb](https://github.com/ophidiarium/cribo/commit/3aafafb4a2f20c6205212d57fbf866472703b93b))

## [0.6.0](https://github.com/ophidiarium/cribo/compare/v0.5.19...v0.6.0) (2025-09-09)


### Features

* centralize __init__ and __main__ handling via python module ([#349](https://github.com/ophidiarium/cribo/issues/349)) ([b6640a2](https://github.com/ophidiarium/cribo/commit/b6640a2cca6cbc30abdd373fa2069312575632b7))


### Miscellaneous Chores

* release 0.6.0 ([45f863b](https://github.com/ophidiarium/cribo/commit/45f863b9cce372979fbf86ec0b8da9b18842c6e4))

## [0.5.19](https://github.com/ophidiarium/cribo/compare/v0.5.18...v0.5.19) (2025-08-28)


### Features

* add Claude Code hook to prevent direct commits to main branch ([bfdcd15](https://github.com/ophidiarium/cribo/commit/bfdcd150483ab6eefc41d31671cf34c52739497b))


### Bug Fixes

* ensure private symbols imported by other modules are exported ([#328](https://github.com/ophidiarium/cribo/issues/328)) ([39c6b1b](https://github.com/ophidiarium/cribo/commit/39c6b1b4887e3e0cc279d3a11bffb29bbc2af2fc))
* ensure tree-shaking preserves imports within used functions and classes ([#330](https://github.com/ophidiarium/cribo/issues/330)) ([30e0229](https://github.com/ophidiarium/cribo/commit/30e0229f919fc2fcef4e26961e2e2d3141d43e59))
* handle lifted globals correctly in module transformation ([#325](https://github.com/ophidiarium/cribo/issues/325)) ([9144c22](https://github.com/ophidiarium/cribo/commit/9144c22bc0e1ee481350fbfc49b01a9cf54d434b))
* handle metaclass dependencies in class ordering ([56a11d3](https://github.com/ophidiarium/cribo/commit/56a11d3994b5dbf461b7049d974e7564a08c648d))
* handle wrapper module imports in function default parameters ([#329](https://github.com/ophidiarium/cribo/issues/329)) ([08d390a](https://github.com/ophidiarium/cribo/commit/08d390a409137e5306fdd002d9e7df13bade86a3))
* improve class dependency ordering for metaclass and class body references ([#327](https://github.com/ophidiarium/cribo/issues/327)) ([bde402e](https://github.com/ophidiarium/cribo/commit/bde402e3fc4234cfa5596d4cc83c8f9eb75af810))
* remove hardcoded http.cookiejar handling with generic submodule import solution ([#331](https://github.com/ophidiarium/cribo/issues/331)) ([cae8ce8](https://github.com/ophidiarium/cribo/commit/cae8ce808e690485232229299b42406ede84d278))
* remove unnecessary statement reordering and self-referential wildcard imports ([c4b8e5f](https://github.com/ophidiarium/cribo/commit/c4b8e5f921b5d6b740c65037e065885253880953))
* remove unused code to resolve clippy warnings ([e6c759b](https://github.com/ophidiarium/cribo/commit/e6c759b5fb3f2efb2c9bea14c8ab536c92b18298))
* skip self-referential re-export assignments in bundler ([b1e1903](https://github.com/ophidiarium/cribo/commit/b1e19038ac1cd8e8365d7fcaff7c4304ca7336dd))
* update namespace creation detection for stdlib proxy ([#336](https://github.com/ophidiarium/cribo/issues/336)) ([a921544](https://github.com/ophidiarium/cribo/commit/a9215449ef5cf38e326897b911406aa5482fafa7))

## [0.5.18](https://github.com/ophidiarium/cribo/compare/v0.5.17...v0.5.18) (2025-08-21)


### Bug Fixes

* add module namespace assignments for wildcard imports in wrapper inits ([#318](https://github.com/ophidiarium/cribo/issues/318)) ([e94f158](https://github.com/ophidiarium/cribo/commit/e94f158d9e2f57c4910de73f1dc7a5eb182f9c2d))
* handle circular dependencies with __version__ module imports ([#314](https://github.com/ophidiarium/cribo/issues/314)) ([e2fbfe9](https://github.com/ophidiarium/cribo/commit/e2fbfe99a7b583fb7c5e9d591b1bd2073012e9d7))
* include explicitly imported private symbols in circular dependencies ([#312](https://github.com/ophidiarium/cribo/issues/312)) ([8c18c8a](https://github.com/ophidiarium/cribo/commit/8c18c8a5de6599e0bda04d00e3a53c3d4192b9c1))
* preserve symbols accessed dynamically via locals/globals with __all__ ([#317](https://github.com/ophidiarium/cribo/issues/317)) ([715eb1d](https://github.com/ophidiarium/cribo/commit/715eb1d3731d3b5b983996a46b38d4db9509ec73))

## [0.5.17](https://github.com/ophidiarium/cribo/compare/v0.5.16...v0.5.17) (2025-08-17)


### Features

* add httpx to ecosystem tests and type_checking_imports fixture ([#306](https://github.com/ophidiarium/cribo/issues/306)) ([bb6401c](https://github.com/ophidiarium/cribo/commit/bb6401cbfad8b0e786e5d5cbfd0cee65b0f89400))


### Bug Fixes

* handle locals() calls in wrapped modules by static analysis ([#308](https://github.com/ophidiarium/cribo/issues/308)) ([50ebf75](https://github.com/ophidiarium/cribo/commit/50ebf751f08ef988fa3cbbba650b94815015f3cc))
* handle wildcard imports from inlined modules that re-export wrapper module symbols ([#311](https://github.com/ophidiarium/cribo/issues/311)) ([b797bd5](https://github.com/ophidiarium/cribo/commit/b797bd5b2d2b2e568bfd67fb25b42de40864d5f9))
* handle wildcard imports in wrapper modules with setattr pattern ([#310](https://github.com/ophidiarium/cribo/issues/310)) ([9d5a4f0](https://github.com/ophidiarium/cribo/commit/9d5a4f0027c4a9df924f89d2acff3a7c5c6fc244))

## [0.5.16](https://github.com/ophidiarium/cribo/compare/v0.5.15...v0.5.16) (2025-08-16)


### Bug Fixes

* preserve aliased imports accessed via module attributes during tree-shaking ([#301](https://github.com/ophidiarium/cribo/issues/301)) ([651ec2a](https://github.com/ophidiarium/cribo/commit/651ec2aca5614cb374fb5058a403f6179f5424b8))
* prevent code generator from referencing tree-shaken symbols ([#305](https://github.com/ophidiarium/cribo/issues/305)) ([78eb188](https://github.com/ophidiarium/cribo/commit/78eb1888e23784588474b86cee8f96dedb2f5d48))

## [0.5.15](https://github.com/ophidiarium/cribo/compare/v0.5.14...v0.5.15) (2025-08-15)


### Bug Fixes

* apply renames to metaclass keyword arguments in class definitions ([#295](https://github.com/ophidiarium/cribo/issues/295)) ([894f45b](https://github.com/ophidiarium/cribo/commit/894f45b9e8ae5a47ace9086af71dd082c9ccbcd7))
* correctly reference symbols from wrapper modules in namespace assignments ([#298](https://github.com/ophidiarium/cribo/issues/298)) ([b78c0a8](https://github.com/ophidiarium/cribo/commit/b78c0a8edcfefaa04daccf43a0c54cb331bdcf49))
* handle wildcard imports correctly for wrapper and inlined modules ([#294](https://github.com/ophidiarium/cribo/issues/294)) ([23d3594](https://github.com/ophidiarium/cribo/commit/23d3594e555b51a0383c355d579980be05d36e5e))
* initialize wrapper modules for lazy imports in inlined modules ([#289](https://github.com/ophidiarium/cribo/issues/289)) ([828c554](https://github.com/ophidiarium/cribo/commit/828c5546044581be31a6926c6347f890f48c9e7f))
* resolve metaclass forward references and tree-shaking for same-module dependencies ([#296](https://github.com/ophidiarium/cribo/issues/296)) ([b564cd6](https://github.com/ophidiarium/cribo/commit/b564cd6d33ac15569e1bfa21929f78bcb88f1136))
* resolve metaclass forward references and tree-shaking for same-module dependencies ([#297](https://github.com/ophidiarium/cribo/issues/297)) ([92aedc4](https://github.com/ophidiarium/cribo/commit/92aedc4d2d39e9fd28f8b0a2e6fe1feb9160b1cd))

## [0.5.14](https://github.com/ophidiarium/cribo/compare/v0.5.13...v0.5.14) (2025-08-14)


### Features

* add bun and dprint installation to copilot setup workflow ([#287](https://github.com/ophidiarium/cribo/issues/287)) ([150ca4c](https://github.com/ophidiarium/cribo/commit/150ca4cc25f8278daffeb1f97b1cd9c59d1a6586))


### Bug Fixes

* copilot setup steps ([9cc2c73](https://github.com/ophidiarium/cribo/commit/9cc2c7361038230e762742a466dc90f6cea11106))
* handle circular dependencies with stdlib-conflicting module names ([#281](https://github.com/ophidiarium/cribo/issues/281)) ([bf65109](https://github.com/ophidiarium/cribo/commit/bf6510908d48aea1e609f4b080a898948a7d7db8))
* preserve stdlib imports and fix module initialization order for wrapper modules ([#283](https://github.com/ophidiarium/cribo/issues/283)) ([0f429ef](https://github.com/ophidiarium/cribo/commit/0f429ef91ba410267559e75579d364a7df483666))
* track all dependencies in side-effect modules during tree-shaking ([#288](https://github.com/ophidiarium/cribo/issues/288)) ([fc3d2a7](https://github.com/ophidiarium/cribo/commit/fc3d2a7e3917c8dae2f44ddba92c363d25f77eff))

## [0.5.13](https://github.com/ophidiarium/cribo/compare/v0.5.12...v0.5.13) (2025-08-13)


### Bug Fixes

* collect dependencies from nested classes and functions in graph builder ([#272](https://github.com/ophidiarium/cribo/issues/272)) ([d0d174b](https://github.com/ophidiarium/cribo/commit/d0d174b89bdabb2fc6a35c46dfc11458c0ef64cc))
* handle stdlib module name conflicts in bundler ([#279](https://github.com/ophidiarium/cribo/issues/279)) ([b245fbe](https://github.com/ophidiarium/cribo/commit/b245fbe87bf7e6de8b426ede9991fb383541d76c))
* improve class ordering for cross-module inheritance ([#277](https://github.com/ophidiarium/cribo/issues/277)) ([ed8fee4](https://github.com/ophidiarium/cribo/commit/ed8fee4c22185f8e67a60384f30958ec38613c3d))
* prevent stdlib module name conflicts in bundled imports ([#275](https://github.com/ophidiarium/cribo/issues/275)) ([6b304fe](https://github.com/ophidiarium/cribo/commit/6b304fee74bf84c9b4a22f341df1bf2c0226da17))

## [0.5.12](https://github.com/ophidiarium/cribo/compare/v0.5.11...v0.5.12) (2025-08-11)


### Bug Fixes

* relative imports being incorrectly classified as stdlib imports ([#267](https://github.com/ophidiarium/cribo/issues/267)) ([bd28eab](https://github.com/ophidiarium/cribo/commit/bd28eabc7b20e7d91c827e2fa5abf5781f46c6bd))

## [0.5.11](https://github.com/ophidiarium/cribo/compare/v0.5.10...v0.5.11) (2025-08-09)


### Features

* use taplo and stable rust ([bc4f080](https://github.com/ophidiarium/cribo/commit/bc4f080391c9f55d28e0f00ba41f9e6894ff8f43))


### Bug Fixes

* install msbuild on windows ([76f8e43](https://github.com/ophidiarium/cribo/commit/76f8e437021eac4910a2d748ac5176aeb6493c9e))

## [0.5.10](https://github.com/ophidiarium/cribo/compare/v0.5.9...v0.5.10) (2025-08-08)


### Features

* implement centralized namespace management system ([#263](https://github.com/ophidiarium/cribo/issues/263)) ([2768882](https://github.com/ophidiarium/cribo/commit/276888220bb742aebff4b3663cc24fd8bedb134b))


### Bug Fixes

* base branch bench missed feature ([afc35dc](https://github.com/ophidiarium/cribo/commit/afc35dc60d3caa58d792432bfa4d546a63530616))
* centralize namespace management to prevent duplicates and fix special module handling ([#261](https://github.com/ophidiarium/cribo/issues/261)) ([c67ab00](https://github.com/ophidiarium/cribo/commit/c67ab007599cb218883695cf4cb4267f91deb495))
* rename bundled_exit_code to python_exit_code for clarity ([76e6c71](https://github.com/ophidiarium/cribo/commit/76e6c714652469d8332e3252bb7ae2798038b781))
* replace cast with try_from for leading_dots conversion ([057f108](https://github.com/ophidiarium/cribo/commit/057f10851d8d3aaf0bc5ba76c07565d6e25f6be3))
* replace unnecessary Debug formatting with Display for paths ([#260](https://github.com/ophidiarium/cribo/issues/260)) ([5b0fe45](https://github.com/ophidiarium/cribo/commit/5b0fe450594072e71b1a2ad1333020f07d1ef8ff))
* use case-insensitive file extension comparison in util.rs ([17133f9](https://github.com/ophidiarium/cribo/commit/17133f9974885bd149283cbd2a4dac105940ff61))

## [0.5.9](https://github.com/ophidiarium/cribo/compare/v0.5.8...v0.5.9) (2025-08-05)


### Bug Fixes

* resolve clippy pedantic warnings for pass-by-value arguments ([#252](https://github.com/ophidiarium/cribo/issues/252)) ([09853c1](https://github.com/ophidiarium/cribo/commit/09853c1fc98b148c446814a1ced881c7db645477))

## [0.5.8](https://github.com/ophidiarium/cribo/compare/v0.5.7...v0.5.8) (2025-08-05)


### Bug Fixes

* handle built-in type re-exports correctly in bundled output ([#240](https://github.com/ophidiarium/cribo/issues/240)) ([770d29a](https://github.com/ophidiarium/cribo/commit/770d29ae11dc79b440dbfb57a4a5dadd4268b515))
* resolve __all__ completely statically ([#247](https://github.com/ophidiarium/cribo/issues/247)) ([b842f10](https://github.com/ophidiarium/cribo/commit/b842f1084f707f51624742ee1f9ab4f65b883c54))
* resolve forward reference errors and redundant namespace creation ([#241](https://github.com/ophidiarium/cribo/issues/241)) ([54813ad](https://github.com/ophidiarium/cribo/commit/54813ad16c20d9ce598c16b57f81a36aeeb35c2c))

## [0.5.7](https://github.com/ophidiarium/cribo/compare/v0.5.6...v0.5.7) (2025-07-31)


### Bug Fixes

* assign init function results to modules in sorted initialization ([#222](https://github.com/ophidiarium/cribo/issues/222)) ([a7755a1](https://github.com/ophidiarium/cribo/commit/a7755a1383029b099b22819a1aa197435cca7eea))
* handle submodules in __all__ exports correctly ([27f92c1](https://github.com/ophidiarium/cribo/commit/27f92c188f6c13c6f2ce89c0860662229008f7de))
* handle submodules in __all__ exports correctly ([#226](https://github.com/ophidiarium/cribo/issues/226)) ([c32d1a8](https://github.com/ophidiarium/cribo/commit/c32d1a8cc7ea9a2fd2e2206b74ed43d09fb777f4))
* include all module-scope symbols in namespace to support private imports ([#225](https://github.com/ophidiarium/cribo/issues/225)) ([a47435d](https://github.com/ophidiarium/cribo/commit/a47435d29eb3c902cc4f1116e737feabe22ea786))
* resolve forward reference errors in hard dependency class inheritance ([#232](https://github.com/ophidiarium/cribo/issues/232)) ([96199bd](https://github.com/ophidiarium/cribo/commit/96199bd9a547f33ffce914c207d433fead63913f))

## [0.5.6](https://github.com/ophidiarium/cribo/compare/v0.5.5...v0.5.6) (2025-07-27)


### Bug Fixes

* **bundler:** handle __version__ export and eliminate duplicate module assignments ([#213](https://github.com/ophidiarium/cribo/issues/213)) ([1148f14](https://github.com/ophidiarium/cribo/commit/1148f144a21b3538e63af7e664441d93f77e568d))
* **bundler:** handle circular dependencies with module-level attribute access ([cd08b97](https://github.com/ophidiarium/cribo/commit/cd08b97ed2f5c638777025f28075961d9fbb94c5))
* **bundler:** handle circular dependencies with module-level attribute access ([#219](https://github.com/ophidiarium/cribo/issues/219)) ([f65e292](https://github.com/ophidiarium/cribo/commit/f65e292eaf91a1d249dfa216c3801ee9ac275fb6))
* **bundler:** prevent duplicate namespace assignments when processing parent modules ([#216](https://github.com/ophidiarium/cribo/issues/216)) ([b1d0873](https://github.com/ophidiarium/cribo/commit/b1d08738a2b7349db96e593660d41a1f46a64bda))
* **bundler:** prevent transformation of Python builtins to module attributes ([#212](https://github.com/ophidiarium/cribo/issues/212)) ([4ccc19b](https://github.com/ophidiarium/cribo/commit/4ccc19bfc52cbab4fe895c7534635f77c840ef0b))
* **bundler:** resolve forward reference issues in cross-module dependencies ([#197](https://github.com/ophidiarium/cribo/issues/197)) ([328995d](https://github.com/ophidiarium/cribo/commit/328995d365dfad2656aac605903cc8994b2c63b1))
* **bundler:** skip import assignments for tree-shaken symbols ([#214](https://github.com/ophidiarium/cribo/issues/214)) ([74874e4](https://github.com/ophidiarium/cribo/commit/74874e44ce41358b7695a36119d54c6781f2368b))
* **bundler:** wrap modules in circular deps that access imported attributes ([#218](https://github.com/ophidiarium/cribo/issues/218)) ([1cd1815](https://github.com/ophidiarium/cribo/commit/1cd18151bbc9f44f92ba6a31c2e4764a4b7d9e35))
* use original name and declare global ([#221](https://github.com/ophidiarium/cribo/issues/221)) ([18bd9e7](https://github.com/ophidiarium/cribo/commit/18bd9e7b612caef3938fd2d3c81dcf8d7f5c4110))

## [0.5.5](https://github.com/ophidiarium/cribo/compare/v0.5.4...v0.5.5) (2025-07-01)


### Features

* add __qualname__ ([8dc09ef](https://github.com/ophidiarium/cribo/commit/8dc09ef5e8cd133e3682f3cf65a6b5cc78e78878))


### Bug Fixes

* **bundler:** apply symbol renames to class base classes during inheritance ([#188](https://github.com/ophidiarium/cribo/issues/188)) ([5a6e229](https://github.com/ophidiarium/cribo/commit/5a6e2291882ad5255e3456a64dad6862c60700fa))
* **bundler:** apply symbol renames to class base classes during inheritance ([#189](https://github.com/ophidiarium/cribo/issues/189)) ([590cc46](https://github.com/ophidiarium/cribo/commit/590cc46f0fc902131d986603080708e76ddbae2a))
* **tree-shaking:** preserve entry module classes and fix namespace duplication ([#186](https://github.com/ophidiarium/cribo/issues/186)) ([ca3bd4e](https://github.com/ophidiarium/cribo/commit/ca3bd4e117223ba838cd1365644613307bba60c8))

## [0.5.4](https://github.com/ophidiarium/cribo/compare/v0.5.3...v0.5.4) (2025-06-30)


### Bug Fixes

* **bundler:** handle conditional imports in if/else and try/except blocks ([#184](https://github.com/ophidiarium/cribo/issues/184)) ([f1b1914](https://github.com/ophidiarium/cribo/commit/f1b1914c798e895f8622c1b22508e287a93b5551))

## [0.5.3](https://github.com/ophidiarium/cribo/compare/v0.5.2...v0.5.3) (2025-06-30)


### Features

* ecosystem tests foundation ([#163](https://github.com/ophidiarium/cribo/issues/163)) ([db530c2](https://github.com/ophidiarium/cribo/commit/db530c2a195c4e0578c7869387939d66b5f58c77))


### Bug Fixes

* ecosystem testing testing advances ([#165](https://github.com/ophidiarium/cribo/issues/165)) ([96b0bcc](https://github.com/ophidiarium/cribo/commit/96b0bcc4e3a2771e687b47f2fcb457510d693cfd))

## [0.5.2](https://github.com/ophidiarium/cribo/compare/v0.5.1...v0.5.2) (2025-06-24)


### Features

* implement static importlib support and file deduplication ([#157](https://github.com/ophidiarium/cribo/issues/157)) ([10a539c](https://github.com/ophidiarium/cribo/commit/10a539cc277918aeefaefcca5c7f9c769e33604e))
* post-checkout hooks ([1045544](https://github.com/ophidiarium/cribo/commit/1045544e45290d5ca160fefe9d00340ac77b7d73))

## [0.5.1](https://github.com/ophidiarium/cribo/compare/v0.5.0...v0.5.1) (2025-06-22)


### Features

* implement tree-shaking to remove unused code and imports ([#152](https://github.com/ophidiarium/cribo/issues/152)) ([529079d](https://github.com/ophidiarium/cribo/commit/529079dba95d39f17a32035f8e75b80422d0eec4))

## [0.5.0](https://github.com/ophidiarium/cribo/compare/v0.4.30...v0.5.0) (2025-06-22)


### Performance Improvements

* **test:** fix slow cli_stdout tests by using pre-built binary ([#149](https://github.com/ophidiarium/cribo/issues/149)) ([6c4c71c](https://github.com/ophidiarium/cribo/commit/6c4c71ce56792ccf71e0ce6778b2b44db64de32b))


### Miscellaneous Chores

* release 0.5.0 ([34cfa41](https://github.com/ophidiarium/cribo/commit/34cfa413069445ee81d5017ffa8d239e804221b9))

## [0.4.30](https://github.com/ophidiarium/cribo/compare/v0.4.29...v0.4.30) (2025-06-16)


### Features

* **ci:** only show rust analyzer for changed files ([#132](https://github.com/ophidiarium/cribo/issues/132)) ([5fca806](https://github.com/ophidiarium/cribo/commit/5fca8064d3491fc2fb227fbc28e4356c66bc5d57))
* **test:** enhance snapshot framework with YAML requirements and third-party import support ([#134](https://github.com/ophidiarium/cribo/issues/134)) ([5f9aba7](https://github.com/ophidiarium/cribo/commit/5f9aba7e210c2095c60a96aecc427c2728ae923c))


### Bug Fixes

* **bundler:** preserve import aliases and prevent duplication in hoisted imports ([#135](https://github.com/ophidiarium/cribo/issues/135)) ([95d28ad](https://github.com/ophidiarium/cribo/commit/95d28adc87ac903a5b09d5c441353b23fbcf3282))
* **test:** enforce correct fixture naming for Python execution failures ([#139](https://github.com/ophidiarium/cribo/issues/139)) ([0353e9b](https://github.com/ophidiarium/cribo/commit/0353e9b3199f4f213b4e84ceced6e583e4adcd50))

## [0.4.29](https://github.com/ophidiarium/cribo/compare/v0.4.28...v0.4.29) (2025-06-15)


### Features

* **ci:** add rust-code-analysis-cli ([c40aff3](https://github.com/ophidiarium/cribo/commit/c40aff371307c65d71ac780795167e1c864932a7))
* implement AST visitor pattern for comprehensive import discovery ([#130](https://github.com/ophidiarium/cribo/issues/130)) ([b73df7d](https://github.com/ophidiarium/cribo/commit/b73df7dcd286f8ced2bfee77eb2e11c022946235))

## [0.4.28](https://github.com/ophidiarium/cribo/compare/v0.4.27...v0.4.28) (2025-06-14)


### Features

* enhance circular dependency detection and prepare for import rewriting ([#126](https://github.com/ophidiarium/cribo/issues/126)) ([a46a253](https://github.com/ophidiarium/cribo/commit/a46a25393b6a9186dd7e74c91b9bc7937c4b4296))


### Bug Fixes

* implement function-scoped import rewriting for circular dependency resolution ([e5813a8](https://github.com/ophidiarium/cribo/commit/e5813a8b5a531bfe640881c8e0bc60a4df01d704)), closes [#128](https://github.com/ophidiarium/cribo/issues/128)

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
