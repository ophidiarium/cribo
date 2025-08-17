# Changelog

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
