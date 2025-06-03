# Publishing **Serpen** CLI to npm with Cross-Platform Binaries – Implementation Plan

## 1. Project Structure & Required Changes

To package Serpen for npm (similar to Rust projects **rspack** and **mako**), we will introduce a new subdirectory (e.g. `npm/`) in the repository to hold npm-related packages. This keeps the Rust crate separate from Node packaging. The structure will include:

- a **“base” npm package** for Serpen (the meta-package users will install, e.g. `serpen`), and
- a **template or generated packages** for each platform-specific binary (e.g. `serpen-linux-x64`, `serpen-linux-arm64`, `serpen-win32-x64`, etc.).

For example, after changes the repo might contain:

```text
npm/ 
├── serpen/                  # Base npm package for Serpen
│   ├── package.json         # package metadata for the meta-package
│   ├── bin/serpen.js        # (Optional) small NodeJS launcher script for CLI
│   └── ...                  # (Optional) e.g. src/index.ts if using TypeScript
├── package.json.tmpl        # Template for platform-specific package.json
└── ... (Rust source remains unchanged)
```

This pattern is used by similar projects. For instance, Orhun’s guide on packaging a Rust CLI for npm shows an `npm/` folder with a base package (`app`) and a `package.json.tmpl` to generate per-OS packages【7†L151-L159】. Mako’s repository similarly uses a monorepo with separate packages for each platform, orchestrated via a workspace (Mako’s main package lists optional platform packages as dependencies)【28†L417-L425】.

**Required changes and additions:**

- **Base package (`npm/serpen`):** Add a `package.json` for the main Serpen npm package. This will include metadata (name `serpen`, version, license, etc.) and an **`optionalDependencies`** field listing each platform-specific package with the same version【7†L193-L200】. For example:

  ```json
  {
      "name": "serpen",
      "version": "X.Y.Z",
      "bin": "./bin/serpen.js",
      "optionalDependencies": {
          "serpen-linux-x64": "X.Y.Z",
          "serpen-linux-arm64": "X.Y.Z",
          "serpen-linux-x64-musl": "X.Y.Z",
          "serpen-linux-arm64-musl": "X.Y.Z",
          "serpen-darwin-x64": "X.Y.Z",
          "serpen-darwin-arm64": "X.Y.Z",
          "serpen-win32-x64": "X.Y.Z",
          "serpen-win32-ia32": "X.Y.Z"
          // (+ optionally serpen-win32-arm64 if targeting Windows ARM64)
      }
  }
  ```

  Every supported OS/architecture combination has a corresponding optional package listed (with version set to the same Serpen version). This mirrors how mako’s main package lists all its platform-specific binaries as optional deps【28†L426-L433】. When a user installs `serpen`, npm/yarn will **automatically install only the matching optional package** for the host platform (thanks to OS/CPU fields in those packages, described below)【40†L73-L81】. This avoids downloading binaries for every platform.

- **Platform package template (`npm/package.json.tmpl`):** This template will be used to generate each platform-specific package’s manifest during the CI build. It contains placeholders for name, version, OS, and CPU. For example:

  ```json
  {
      "name": "${node_pkg}",
      "version": "${node_version}",
      "os": ["${node_os}"],
      "cpu": ["${node_arch}"],
      "bin": {
          "serpen": "./bin/serpen${extension}"
      }
  }
  ```

  Here, `${node_pkg}` will be replaced with the package name (e.g. `serpen-linux-x64`), `${node_os}`/`${node_arch}` with the platform (like `linux` and `x64`), and `${extension}` can be set to `.exe` for Windows binaries (empty for others). We include an OS-specific **`os`** field and **`cpu`** field so that npm knows this package is only valid on that platform【7†L217-L224】. The `bin` field ensures that installing the package will register the contained binary as the `serpen` executable. (In other words, `serpen-linux-x64` package will have a `bin/serpen` file and `"bin": { "serpen": "bin/serpen" }` so that npm links it into the user’s PATH when installed globally or via npx.)

- **Node launcher script (optional):** Since the Serpen CLI has no Node API wrapper (it’s a pure CLI), we don’t need a complex Node binding. However, for a smooth user experience (`npx serpen` or global install), we can provide a small NodeJS script as the `bin` for the base package that simply locates and executes the correct binary. This script (e.g. `npm/serpen/bin/serpen.js`) would use Node’s `require.resolve` or similar to find the installed platform package’s binary and spawn it. For example:

  ```js
  #!/usr/bin/env node
  const { execFileSync } = require('child_process');
  const path = require('path');
  // Determine platform identifier
  const platform = process.platform === 'win32' ? 'win32' : process.platform; 
  const arch = process.arch;
  const pkgName = `serpen-${platform}-${arch}${platform === 'linux' ? (detectMusl() ? '-musl' : '-gnu') : ''}`;
  // Compute binary path inside the optional package
  const binPath = require.resolve(`${pkgName}/bin/serpen${process.platform === 'win32' ? '.exe' : ''}`);
  // Run the binary and forward CLI args
  execFileSync(binPath, process.argv.slice(2), { stdio: 'inherit' });
  ```

  In essence, this script uses the same strategy as Sentry CLI’s wrapper: find the binary path and execute it, inheriting stdio【43†L15-L23】. If the platform-specific package somehow wasn’t installed (e.g. user disabled optional deps), the script can catch the error and print a warning prompting the user to enable optional dependencies or install the correct binary manually【42†L25-L33】【42†L53-L60】. (This Node wrapper is minimal and only serves to launch the Rust binary – no heavy Node logic is involved, aligning with the “no complex Node wrapper” requirement.)

_If you prefer not to include a Node launch script at all:_ An alternative is to rely on npm’s bin linking of the platform packages. In that case, the base package’s `package.json` would not declare a `bin` itself; instead each platform package’s `bin` entry would cause `npx` or global installs to expose `serpen`. However, because the base meta-package is the one being installed, its dependencies’ binaries might not automatically be linked globally by npm. The safest approach is to provide the stub launcher in the base package as above (this is how both **rspack** and **@sentry/cli** handle it, ensuring `npx serpen` works out of the box by running the Rust binary【43†L15-L23】).

## 2. Configuration Files & Scripts for npm Publishing

With the project structured for npm, we need to set up configuration and scripts to produce and publish the packages:

- **NPM package manifests:** We already prepared `package.json` for the base package and a template for platform packages. Ensure the base `package.json` includes relevant fields (name, description, homepage/repo, license, keywords, etc.) and the optionalDependencies map to all supported platforms (as determined by Serpen’s existing release matrix). Each platform package will get a generated `package.json` from the template with appropriate `name`, `version`, `os`, `cpu`, and `bin` fields. The `os`/`cpu` fields are critical – they restrict installation to the intended platform, so npm/yarn will skip incompatible ones. For example, `serpen-linux-arm64` will have `"os": ["linux"], "cpu": ["arm64"]`, so it only installs on Linux ARM64 hosts. For Linux, we will likely provide both _gnu_ and _musl_ variants. Because npm doesn’t have a direct “libc” selector, our strategy is to publish e.g. `serpen-linux-x64-gnu` and `serpen-linux-x64-musl` both with `"os": ["linux"], "cpu": ["x64"]`. This means **both** might be fetched on Linux x64. Our base launcher can detect at runtime which one to prefer – or we could instruct Alpine users to install with `--ignore-optional` for gnu package. Many projects tolerate the redundant download for completeness. Mako, for instance, publishes separate GNU and MUSL packages for Linux x64/arm64【28†L426-L433】.

- **Versioning**: We will keep the npm package version in sync with the Serpen crate version (and PyPI version). The GitHub Actions workflow can inject the version number (for example, derive it from the git tag or Cargo.toml). In Orhun’s example, they export `RELEASE_VERSION` from the tag and use it when generating package.json files. We can do similarly, so that `${node_version}` in our template is set to the new release version, ensuring all npm packages (binary packages and the meta-package) use the same version string.

- **npm Publish Config:** In the base package’s `package.json`, you might include a `"publishConfig": { "access": "public" }` if using an npm organization scope or if the packages are under a scope. Since `serpen` is unscoped (public), this is not strictly necessary – we can pass `--access public` in the publish command for scoped packages. Each platform package will be published as public as well.

- **Build scripts:** If using TypeScript for the launcher (`src/index.ts` in base), include a `tsconfig.json` and devDependencies (TypeScript, maybe ESLint, etc.) in the base package. Provide an npm script like `"build": "tsc"` and possibly configure `prepare` script to run `build` on publish. Orhun’s base package uses a TypeScript setup with build/lint scripts, and runs `yarn install && yarn build` before publishing. If we keep the launcher in plain JavaScript, we can skip the compile step – just ensure the `bin/serpen.js` is included in the package files.

- **.npmignore or files:** Ensure the npm packages only include the necessary files. For platform packages, by generating them in CI, we control their content (just the binary and package.json, plus maybe a README). For the base package, if the `npm/serpen` directory contains source files, use a `.npmignore` or `"files"` field so that only the built output (e.g. the `bin/` folder or `lib/` folder with compiled JS) is published. This keeps the package lean.

- **Node scripts for install (if any):** _Optional:_ We might include a small `postinstall` script in the base package as a fallback to handle scenarios where optionalDependencies failed (e.g. user used `--no-optional`). The Sentry blog recommends using both optionalDependencies and a postinstall download as a backup. For Serpen, a simpler approach could be: on postinstall, check if the expected binary exists; if not, print an error or attempt to download the correct binary from a known URL (for example, from the GitHub Releases). This adds robustness. However, if we expect most users to allow optional deps, we can omit the download step initially. We will document clearly that enabling optional dependencies is required to install Serpen via npm (and our launcher will warn if it can’t find a binary).

In summary, the configuration boils down to preparing **consistent package.json files** for one main package and multiple platform-specific ones, and a possible Node stub script. This setup follows the model used by projects like **esbuild** and **@rspack/core/cli**, which distribute prebuilt binaries via npm optional dependencies.

## 3. Cross-Platform Binary Compilation

Next, we leverage Serpen’s existing GitHub Actions CI matrix to build the Rust binary for each target platform. The goal is to reuse the same matrix defined in the current release workflow (as in `.github/workflows/release.yml`) that you use for PyPI wheels, so we don’t duplicate work. Each platform in the matrix should produce a Serpen binary which we will package for npm.

**Supported targets:** Based on the current release matrix (and common targets of rspack/mako), we will include:

- **Linux (x86_64 and ARM64)** – build both glibc (`unknown-linux-gnu`) and musl (`unknown-linux-musl`) variants to cover most Linux distros (the glibc builds will cater to Ubuntu/Debian/Fedora etc., while musl builds cover Alpine Linux).
- **macOS (x86_64 and ARM64)** – build for both Intel and Apple Silicon Macs.
- **Windows (x64)** – build the Windows MSVC 64-bit binary. If feasible, also build **Windows 32-bit (i686)** and **Windows ARM64**, as Rust and GitHub Actions support those targets (32-bit Windows is less common now, so this is optional). Rspack, for example, produces x86_64, i686, and ARM64 Windows binaries. If the existing PyPI release matrix doesn’t include 32-bit or ARM64 Windows, we can omit them for npm to save time.

Each of these targets corresponds to one optional npm package (named accordingly, e.g. `@serpen/darwin-arm64` for Mac M1, etc.). We should ensure the target triples in Rust match the naming convention we use for packages. For instance:

- Rust target `x86_64-unknown-linux-gnu` -> npm package `@serpen/linux-x64-gnu`
- Rust target `aarch64-unknown-linux-musl` -> npm package `@serpen/linux-arm64-musl`
- Rust target `x86_64-pc-windows-msvc` -> npm package `@serpen/win32-x64` (assuming `-msvc` is implicit in name or we can include it for clarity)

_(You can choose a consistent naming scheme. Mako uses `win32-x64-msvc` in package names, whereas some projects omit the “msvc”. The key is that names match what our base optionalDependencies expect.)_

**GitHub Actions build strategy:** We will update the release workflow to build these targets. Likely, your current workflow already builds wheels via maturin, possibly using `maturin build` or `publish` which invokes cargo under the hood. To get standalone binaries:

- We can add a step in each matrix job **after** building the wheel to copy out the binary. For example, if maturin produces an executable (perhaps in the wheel or target folder), we use that. If not, we can explicitly run `cargo build --release --target <TRIPLE>` for each platform. Since maturin already compiled the code, this might be redundant on the same runner – but for clarity, we might just invoke cargo directly to get a clean binary. (Ensure the Rust toolchain has the appropriate targets installed for cross-compilation if needed.)

- If cross-compiling is needed (e.g. building Linux ARM64 on a x86_64 Linux runner, or Windows 32-bit on a 64-bit runner), consider using **cross-compilation tools**. For Linux targets, we can use `actions-rs/toolchain` with `target: aarch64-unknown-linux-gnu` and perhaps `cross` for musl if needed. Orhun’s example uses `actions-rs/cargo` with `use-cross: true` for Linux targets to build ARM binaries on an x64 runner. Alternatively, use Docker images (e.g. the official Rust musl builder or cross images) for musl builds. For Windows ARM64, since GitHub Actions doesn’t have ARM Windows runners, cross-compiling from Windows x64 using Rust’s `aarch64-pc-windows-msvc` target is an option (ensuring the Visual Studio tools are present).

- Each job should produce a single executable file (e.g. `serpen` or `serpen.exe`). We will need to **upload or pass this binary to the packaging step**. One approach (used by many projects) is to do the packaging and publishing **within the same job** right after building the binary, so we don’t have to transfer artifacts between jobs. This is what we’ll do (see next section).

Ensure that for each target we set appropriate file permissions (the Linux/macOS binaries should be executable – by default they will be). Also consider stripping symbols to reduce size (Rust’s release build already strips unneeded symbols, but you can run `strip` on Linux binaries for example, if not already done).

In summary, we’ll utilize the GH Actions matrix to compile Serpen for all required OS/arch combos. This reuses the existing matrix (and runners) defined for releases. For example, if the current release matrix defines jobs for `ubuntu-latest`, `macos-latest`, `windows-latest`, etc., we will expand it to also include different architectures (using `strategy.matrix`) or use cross-compilation as appropriate. The end result: each job outputs a Serpen binary ready to be bundled.

## 4. Packaging & Uploading Platform-Specific Binaries to npm

Once a Serpen binary is built in a CI job, that same job will bundle and publish the corresponding npm package (e.g. `serpen-linux-x64`). This is done for each platform in parallel. Here’s a step-by-step breakdown for one platform job:

1. **Prepare npm environment:** Install Node.js on the runner (using `actions/setup-node@v3`) and authenticate to npm. For example:

   ```yaml
   - uses: actions/setup-node@v3
     with:
       node-version: '18'
       registry-url: 'https://registry.npmjs.org'
   ```

   This ensures we have `npm` available. Authentication can be handled by setting the `NODE_AUTH_TOKEN` env var to an npm token (we’ll use a secret for this) or by `npm set //registry.npmjs.org/:_authToken=${{ secrets.NPM_TOKEN }}`. In our case, we’ll use the environment variable method during `npm publish`.

2. **Generate the platform package folder:** Use the template to create a package for this specific OS/arch. For instance, in the job script:

   - Determine normalized OS and architecture names. We can use environment info or matrix variables. E.g., for a matrix entry we might have a name like `"linux-x64-gnu"`. Orhun’s workflow derives `node_os` and `node_arch` by splitting a matrix name string. We can also map from runner OS: if `runs-on: macos-latest` and arch is x64, then `node_os=darwin`, `node_arch=x64`; if `runs-on: windows-latest`, use `win32` for `node_os` (or “windows” if we decided that naming). Essentially:
     - Use `linux`, `darwin`, or `win32` as OS strings (npm expects `"win32"` for Windows in the `os` field).
     - Use `x64`, `arm64`, or `ia32` as the CPU (npm uses `ia32` for 32-bit x86).

   - Form the package name, e.g.: `serpen-${node_os}-${node_arch}` and append `-musl` or `-gnu` if applicable for Linux. (We set this in `${node_pkg}` environment variable.)

   - Create a directory for the package, e.g. `$node_pkg/`. Inside it, make a subdirectory `bin/`.

   - Fill out the `package.json`: we can use a tool like `envsubst` or simple string replacement. For example, Orhun’s CI step uses `envsubst` to substitute placeholders in `package.json.tmpl` and writes the result to the package folder. In our case:
     ```bash
     export node_pkg="@serpen/linux-x64-gnu"
     export node_os="linux"
     export node_arch="x64"
     export node_version="0.1.0"   # (Set from release version env)
     envsubst < npm/package.json.tmpl > $node_pkg/package.json
     ```
     This produces `serpen-linux-x64-gnu/package.json` with `"name": "serpen-linux-x64-gnu", "os": ["linux"], "cpu": ["x64"], "version": "0.1.0", ...`. Verify the fields are correct (especially that OS/CPU match this target).

   - Copy the compiled binary into the package’s `bin/` directory. For example:
     ```bash
     cp path/to/serpen-binary $node_pkg/bin/serpen${EXT}
     ```
     where `${EXT}` is `.exe` for Windows or empty for others. In Orhun’s workflow, they adjust the binary name for Windows (`bin="${bin}.exe"`) and copy it. We should do similarly. After this, the package folder contains the binary (e.g. `bin/serpen` or `bin/serpen.exe`) and the `package.json`. _(We may also include a small README in each package to satisfy npm if needed, but it’s optional.)_

3. **Publish the platform package to npm:** With the package folder ready, publish it using `npm publish`. We change into that folder and run:
   ```bash
   cd "$node_pkg"
   npm publish --access public
   ```
   The `--access public` flag is only required for scoped packages; if `serpen` is unscoped on npm, publishing defaults to public. You can include it for safety if using a scope or skip it if unscoped. Ensure the npm authentication token is available in `NODE_AUTH_TOKEN` (in GH Actions, setting this env var will let `npm publish` use it automatically). We’ll store an npm token in the repository secrets (e.g. `NPM_TOKEN`) with publish rights to the `serpen` package name.

Each matrix job will perform these steps. In practice, we can script the generation and publish in one shell script block for simplicity (as shown in Orhun’s example). The jobs run in parallel, publishing all the binary packages for Linux, Windows, macOS, etc.

**Important:** The platform-specific packages **must be published before** the meta-package. This is because the meta-package’s optionalDependencies refer to specific versions of those packages – if they don’t exist on npm, installation of the meta-package will fail. Our workflow ensures this by doing all binary package publishes first, then the meta-package (see next section).

We should also handle potential failures – e.g., if one package fails to publish (network glitch, etc.), the workflow should ideally fail before publishing the meta package. This way we don’t end up with a meta-package that points to non-existent binary packages. We might use the job dependency ordering to enforce that (e.g. have a job that publishes the meta package depend on the success of all matrix binary jobs).

## 5. Publishing the Meta Package (`serpen`) and Selecting Binaries by Host OS

After all the platform-specific packages are uploaded, the final step is to publish the **Serpen meta-package** (the main `serpen` package on npm). This package doesn’t contain the binary itself, but it **pulls in the appropriate binary** for the user’s platform via the optionalDependencies mechanism.

**Meta-package contents recap:** The base package’s `package.json` lists each `serpen-<platform>` package under `optionalDependencies` with the same version. It may also include our launcher script in the `bin` field. For example:

```json
{
    "name": "serpen",
    "version": "0.1.0",
    "description": "...",
    "bin": {
        "serpen": "bin/serpen.js"
    },
    "optionalDependencies": {
        "@serpen/linux-x64-gnu": "0.1.0",
        "@serpen/linux-x64-musl": "0.1.0",
        "@serpen/linux-arm64-gnu": "0.1.0",
        "@serpen/linux-arm64-musl": "0.1.0",
        "@serpen/darwin-x64": "0.1.0",
        "@serpen/darwin-arm64": "0.1.0",
        "@serpen/win32-x64": "0.1.0",
        "@serpen/win32-ia32": "0.1.0"
    }
}
```

When a user runs `npm install serpen` (or `npm i -g serpen` or `npx serpen`), the npm client will examine those optional deps. Thanks to the `os`/`cpu` fields in each platform package’s manifest, npm will **only download the one(s) matching the current OS and architecture**. For example, on a macOS ARM64 machine, it will fetch `serpen-darwin-arm64@0.1.0` (and skip the others as “unsupported” optional deps, possibly logging warnings). On a 64-bit Windows PC, it will fetch `serpen-win32-x64@0.1.0`, etc. This behavior happens automatically during installation.

To publish the meta-package, we add a **final job in the workflow** (after all binary jobs). This job will:

- Checkout the repository (to get the latest package.json and any built launcher script).
- Ensure the base package is ready. If we used TypeScript for the launcher, run the build (e.g. `npm ci && npm run build` inside `npm/serpen` directory) so that `bin/serpen.js` (or `lib/index.js`) exists. If the launcher is plain JS and already committed, this step is not needed beyond maybe an `npm install` to grab any production dependencies (though in our case, there are none aside from the optional deps, which are not installed at publish time).
- Publish the package: e.g.

  ```bash
  cd npm/serpen
  npm publish --access public
  ```

  (Again, ensure `NPM_TOKEN` is set. Also, we might want to run `npm install` or `yarn install` before publish if a build step is needed or to update the lockfile – Orhun’s workflow runs `yarn install` mainly to ensure the optional deps versions are locked in yarn.lock before publishing, but that’s optional. We can publish directly as long as the `package.json` is correct and the built files are present.)

Once this meta-package is published, users can install Serpen via npm:

- Running `npm install -g serpen` will place the `serpen` executable in their PATH (global node_modules/.bin). The installation will have pulled down only the needed binary package (plus the small meta-package overhead).
- Using `npx serpen` will fetch the `serpen` package at runtime, which in turn fetches the right binary, and then our `bin/serpen.js` launcher will execute the binary immediately. (For example, `npx serpen --help` would download `serpen`@latest, which brings in e.g. `serpen-linux-x64-gnu@latest`, and the launcher script then runs the actual Rust CLI.)

**Verifying the selection logic:** We should test the published packages by installing on different platforms to ensure that exactly one binary gets installed and runs correctly. If the optional dependency for some reason doesn’t install (e.g. user has `optional = false` in npm config), our base package’s postinstall or launcher can detect the absence and show a clear message. But in normal cases, npm will log a message like “Skipping unsupported optional dependency: serpen-linux-arm64-musl” on platforms where it’s not needed, and only the correct one will be marked as installed.

By following this pattern, we effectively create a **meta-package** (sometimes called a “proxy” or “wrapper” package) that _dynamically pulls in the correct native binary_. This approach is proven and used by projects like Sentry CLI, esbuild, Prisma, rspack, etc. Users of Serpen can now install it via npm without needing a Rust toolchain – the prebuilt binaries will be readily available.

Finally, we ensure the GitHub Actions workflow is triggered on releases (e.g. when you push a new git tag). It will build and upload all binaries and the npm packages as outlined, in one automated process. This way, publishing to npm can be done alongside the existing PyPI release process. The overall release flow will be:

1. Developer bumps version and creates a new release (tag).
2. GitHub Actions runs the release workflow:
   - Builds wheels and uploads to PyPI (existing process via maturin).
   - **In parallel** (or sequentially), builds each target’s binary and publishes `serpen-<platform>` packages to npm.
   - After all binaries are published, publishes the `serpen` meta-package to npm.
3. Verify on npm registry: the `serpen` package should show the new version, and the platform packages should be listed as dependencies (you can also see each `serpen-foo-bar` package on npm).

By reusing the matrix and CI infrastructure, we minimize extra build time – the same compiled artifacts can be used for both PyPI and npm releases where possible. This plan ensures cross-platform availability on npm with minimal friction, following the example of **rspack** and **mako** to deliver a seamless install experience for Serpen’s users.

**Sources:**

- Orhun’s blog on _Packaging Rust applications for NPM_ – inspiration for project structure, optional dependency template, and CI scripting.
- Sentry Engineering blog – best practices for distributing platform-specific binaries via npm (using optionalDependencies and a postinstall fallback).
- Rspack and Mako examples – real-world projects using multiple npm binary packages. Mako’s main package lists a matrix of optional deps for each OS/arch (Linux musl vs gnu, Windows, macOS), and Rspack documents the range of binaries they release (Linux x64/arm64, macOS x64/arm64, Windows x86/x64/arm64), which guided our target selection.
