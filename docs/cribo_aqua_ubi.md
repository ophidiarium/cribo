# Updating Cribo for Aqua and UBI CLI Installation

This document explains how to modify the **Cribo** project’s GitHub Actions release workflow to support CLI installation via **Aqua** and **UBI**. We will analyze the current `release.yml` workflow, identify necessary changes, and provide step-by-step instructions to implement multi-platform binary builds, checksums, and release assets. Additionally, we include an example Aqua registry entry and a brief UBI compatibility note with a documentation snippet. This guide is structured for clarity and actionability, enabling a developer to update Cribo’s release process confidently.

## Analyzing the Current Release Workflow

The existing `release.yml` (GitHub Actions workflow) is responsible for publishing releases. In its current form, the workflow likely triggers on a new tag (or release) and runs a build on a single platform (e.g., Ubuntu). Key steps might include checking out the code and building or packaging the project. **However, to support Aqua and UBI CLI installation, we identified several gaps in the current workflow:**

- **Single-Platform Build:** The workflow builds Cribo for one platform (e.g., only Linux). It doesn’t produce binaries for other operating systems like Windows or macOS. This means users on other platforms cannot directly download a pre-built CLI executable.
- **No Archived Binaries:** The workflow does not archive the CLI binary into `.zip` or `.tar.gz` packages. Aqua and UBI rely on downloadable binary archives, so the lack of packaged artifacts is a limitation.
- **No Checksum Generation:** There is no step to generate SHA256 checksums for the release binaries. Checksums are important for integrity verification and are often used by package managers or installers for security.
- **No Release Assets Upload:** The workflow currently doesn’t attach built binaries to the GitHub Release. Users cannot easily fetch the CLI from the Releases page, and Aqua/UBI cannot find any assets to download. The release might be created with release notes, but without binary assets for each platform.

**In summary**, the current release workflow needs to be extended to build Cribo’s CLI for all supported platforms, package those binaries, generate checksum files, and upload everything as assets to the GitHub Release. These enhancements will enable installation via Aqua and UBI. Below, we detail the required modifications, keyed to the relevant workflow sections and steps.

## Summary of Required Workflow Changes

To achieve Aqua and UBI support, we will make the following changes to `.github/workflows/release.yml`:

- **Add a Multi-Platform Build Matrix:** Modify the workflow to build Cribo for multiple operating systems and CPU architectures. This involves introducing a **matrix** strategy (covering Linux, macOS, and Windows, at minimum) in the build job. Each matrix entry will compile the CLI for a specific OS/architecture combination.
- **Archive Binaries per Platform:** After compiling the Cribo binary in each matrix job, package it into an archive file. Use **`.tar.gz`** for Linux/macOS and **`.zip`** for Windows for compatibility. Name each file in a consistent, Aqua-friendly format (including OS and arch in the filename).
- **Generate SHA256 Checksums:** For each binary archive, generate a SHA256 checksum file. This can be done with built-in tools (`sha256sum` or `shasum` on Unix, and PowerShell’s hashing cmdlets on Windows). The checksum files will also be uploaded to the release.
- **Attach Assets to the GitHub Release:** Update the workflow to upload all the binary archives and checksum files as assets to the GitHub Release. This may involve using GitHub’s release actions (e.g., `actions/create-release` and `actions/upload-release-asset`) or the GitHub CLI. We will ensure each asset is properly named and has the correct content type.

With these changes, each release will contain pre-built CLI binaries for all target platforms, along with their checksums. Aqua and UBI can then fetch the appropriate binary for a user’s system.

## Step-by-Step Implementation Instructions

Following is a step-by-step guide to implement the above changes. Each step corresponds to a key area: building binaries, generating checksums, uploading assets, and naming conventions. We include exact YAML snippets and line-by-line insertions for clarity.

1. **Introduce a Multi-Platform Build Matrix and Compile Binaries:**\
   In the workflow YAML, locate the job that builds the project (e.g., `jobs.build` or similar). Modify it to use a matrix strategy for multiple platforms. For example, add a `strategy.matrix` section under the job, and define the target OS and architectures. You can also map friendly names for use in file names:

   ```yaml
   jobs:
     build:
       name: Build Cribo CLI
       runs-on: ${{ matrix.os }}
       strategy:
         matrix:
           include:
             - os: ubuntu-latest # Linux x86_64
               target_os: linux
               target_arch: amd64
             - os: ubuntu-latest # Linux ARM64
               target_os: linux
               target_arch: arm64
             - os: macos-latest # macOS (Darwin) x86_64
               target_os: darwin
               target_arch: amd64
             # (Optional: macOS ARM64 build on an ARM runner or cross-compile)
             - os: windows-latest # Windows x86_64
               target_os: windows
               target_arch: amd64
       steps:
         - name: Check out code
           uses: actions/checkout@v3
         - name: Set up Go (if Cribo is written in Go)
           uses: actions/setup-go@v4
           with:
             go-version: '1.20'
   ```

   In the above snippet, we **inserted a matrix** with combinations for Linux, macOS, and Windows (adjust the language setup step according to Cribo’s implementation – e.g., use `actions/setup-node` for Node.js, `actions-rs/toolchain` for Rust, etc.). Each matrix entry provides `matrix.target_os` and `matrix.target_arch` which we’ll use for naming. The runner `runs-on` is set to the appropriate OS for native building on that platform. If Cribo is a compiled language (Go/Rust), you can build natively on each runner or cross-compile as needed. For example, in Go you can set environment variables `GOOS` and `GOARCH` to cross-compile; in Rust you might use `--target`. Assuming native build on each runner:

   ```yaml
   - name: Build Cribo binary
     env:
       GOOS: ${{ matrix.target_os }} # Only needed if cross-compiling in Go
       GOARCH: ${{ matrix.target_arch }} # (On native runners, Go builds use host OS by default)
     run: |
       # Compile the CLI
       go build -o cribo${{ matrix.target_os == 'windows' && '_'+matrix.target_os || '' }}                      ./cmd/cribo
       # Explanation: if building on Windows, we append "_windows" or use .exe in name below
   ```

   **Explanation:** The above `go build` command is just an example. It compiles the Cribo CLI. For Windows, you would want the binary to have a `.exe` extension. You can handle this by using a conditional or simply adding `.exe` after building (e.g., rename the file). The key is to produce an output binary named `cribo` for Linux/macOS and `cribo.exe` for Windows. Adjust the build command to your project’s language/toolchain. (For instance, if Cribo is a Rust project, use `cargo build --release` and find the binary in `target/release`; if it’s a Node project, you might use a bundler to create an executable.)

2. **Archive the CLI Binaries (Packaging per Platform):**\
   After the binary is built for a platform, add steps to **archive** it into a compressed file. We will use **`.tar.gz` for Unix-like systems** and **`.zip` for Windows**. This ensures maximum compatibility (Windows users can easily unzip, and tar.gz is standard for Linux/macOS). Use the `matrix.target_os` and `matrix.target_arch` variables to name the archives. Insert the following steps *after* the build step in the workflow:

   ```yaml
   - name: Package CLI binary
     if: ${{ matrix.target_os != 'windows' }}
     run: |
       # Tar and gzip the binary for Linux/macOS
       tar -czf cribo_${{ matrix.target_os }}_${{ matrix.target_arch }}.tar.gz cribo
   - name: Package CLI binary (Windows)
     if: ${{ matrix.target_os == 'windows' }}
     run: |
       # Use PowerShell to create a zip on Windows
       powershell Compress-Archive -Path cribo.exe -DestinationPath cribo_${{ matrix.target_os }}_${{ matrix.target_arch }}.zip
   ```

   These steps create an archive named, for example, **`cribo_linux_amd64.tar.gz`**, **`cribo_darwin_amd64.tar.gz`**, or **`cribo_windows_amd64.zip`**. The naming includes the OS and architecture, which is crucial for Aqua/UBI. (We include the OS as `darwin` for macOS to align with Go’s GOOS and common naming conventions. This will be mapped to “Darwin” in Aqua’s registry entry.) Each archive contains the single `cribo` executable (with the proper extension on Windows). **Ensure the archive does not introduce an extra directory level** – it should ideally contain the binary at the root. In our steps above, we directly archive the file, so the archive structure is flat (this simplifies Aqua configuration).

   *Why this naming?* Aqua and UBI will look for keywords in filenames to identify the correct download for a given platform. By including `linux`, `darwin` (for macOS), or `windows` along with `amd64`/`arm64` in the file name, we make it easy for installers to pick the right asset. For example, UBI will filter release assets by OS and CPU architecture substrings (like “linux” and “amd64” for a Linux x86_64 system). Our naming convention ensures each asset is unambiguous.

3. **Generate SHA256 Checksums for Each Archive:**\
   For security and verification, add a step to compute the SHA256 checksum of each archive. This step should run after the packaging step (for each platform). We will produce a `.sha256` file alongside each archive. Insert the following:

   ```yaml
   - name: Compute SHA256 checksum
     shell: bash
     run: |
       # Compute SHA256 and store in a file (platform-specific command)
       if [[ "$RUNNER_OS" == "Windows" ]]; then
         certutil -hashfile cribo_${{ matrix.target_os }}_${{ matrix.target_arch }}.zip SHA256 > checksum.txt
         findstr /v "hash" checksum.txt > cribo_${{ matrix.target_os }}_${{ matrix.target_arch }}.sha256
       else
         sha256sum cribo_${{ matrix.target_os }}_${{ matrix.target_arch }}.tar.gz > cribo_${{ matrix.target_os }}_${{ matrix.target_arch }}.sha256
       fi
   ```

   **Explanation:** On Linux/macOS runners, the step uses `sha256sum` to generate a checksum and writes it to a file named `cribo_<os>_<arch>.sha256`. On Windows, we use the built-in `certutil` to get the SHA256 (redirecting output and filtering out extraneous text) and save it similarly. Each `.sha256` file will contain the hash and the filename. For example, `cribo_linux_amd64.sha256` might contain:
   ```
   e3b0f... (hash)  cribo_linux_amd64.tar.gz
   ```
   These checksum files will be uploaded to the release along with the binaries. They allow users (or automated tools) to verify the integrity of the downloaded archives. Aqua and UBI may not require these for installation, but providing them is good practice for security-conscious users.

4. **Upload Archives and Checksums as Release Assets:**\
   Now that we build and package all binaries, the workflow must upload them to the GitHub Release. This can be done in two ways: (a) using a **separate job** to create a release and then uploading from each matrix job, or (b) collecting artifacts and uploading in one job. We will outline a robust approach using two jobs for clarity.

   - **Create or Update the GitHub Release:** If the workflow isn’t already creating a Release, add a step (or job) at the end of the process to create one. You can use the official `actions/create-release@v1` action. Place this **after** the build job, or in a separate job that runs once. For example, as a new job in the workflow (outside the matrix):

     ```yaml
     jobs:
       create_release:
         name: Create GitHub Release
         runs-on: ubuntu-latest
         needs: build # ensure all builds are done
         outputs:
           upload_url: ${{ steps.create.outputs.upload_url }}
         steps:
           - name: Create Release
             id: create
             uses: actions/create-release@v1
             env:
               GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
             with:
               tag_name: ${{ github.ref }} # e.g. refs/tags/v1.2.3
               release_name: ${{ github.ref_name }} # e.g. v1.2.3
               draft: false
               prerelease: false
     ```

     This job will create a new GitHub Release (if it doesn’t exist) for the tag that triggered the workflow. We capture the release `upload_url` as an output (using `id: create` and the action’s `upload_url` output) so it can be used by subsequent steps. The `needs: build` ensures this runs only after the matrix build job completes successfully.

   - **Upload Assets to the Release:** For each file (archives and checksums) produced in the build job, we use the GitHub Releases API to attach them. We can do this in a loop or by explicit steps. One approach is to perform the upload from within each matrix job. Alternatively, continue in the `create_release` job by downloading artifacts. For simplicity, we can have the matrix jobs upload their artifact directly, since they already know the file names. We’ll use `actions/upload-release-asset@v1` for this purpose. Add the following as the **last step** in the **build job** (so it runs for each matrix instance):

     ```yaml
     - name: Upload assets to Release
       uses: actions/upload-release-asset@v1
       env:
         GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
       with:
         upload_url: ${{ needs.create_release.outputs.upload_url }}
         asset_path: cribo_${{ matrix.target_os }}_${{ matrix.target_arch }}.${{ matrix.target_os == 'windows' && 'zip' || 'tar.gz' }}
         asset_name: cribo_${{ matrix.target_os }}_${{ matrix.target_arch }}.${{ matrix.target_os == 'windows' && 'zip' || 'tar.gz' }}
         asset_content_type: ${{ matrix.target_os == 'windows' && 'application/zip' || 'application/gzip' }}
     ```

     Let’s break down this snippet:
     - It uses the `actions/upload-release-asset` action to upload a file to the release. The `upload_url` comes from the output of the `create_release` job (as configured above). We reference it via `needs.create_release.outputs.upload_url` (because the build job has `needs: create_release`).
     - `asset_path` is the path to the file we want to upload. We use a ternary expression to handle the extension: if the OS is windows, use `.zip`, otherwise use `.tar.gz`. This path will point to the archive we created (e.g., `cribo_linux_amd64.tar.gz`).
     - `asset_name` is how the file will be named in the GitHub release. We set it identical to the file name (this is what users will see and download).
     - `asset_content_type` is set appropriately (zip files use `application/zip`; tar.gz uses `application/gzip`).

     This step will run in each matrix job, uploading that job’s archive to the release. By using the unique names (which include OS/arch), we ensure no collisions and each asset is labeled clearly for users. The GitHub Actions runner will log an outcome for each upload. After this, the release should have all the binaries attached.

     **Note:** If the workflow is triggered by a `release` event (instead of a tag push), you can simplify by using `github.event.release.upload_url` directly without needing a separate create step. In that scenario, ensure `actions/upload-release-asset` uses the event’s `upload_url`. But the above approach (create then upload) is generally applicable and is aligned with GitHub’s examples for multi-platform releases.

5. **Ensure Aqua-Compatible Filenames and Structure:**\
   We have touched on naming conventions in earlier steps, but it’s important to finalize the **filenames** and archive **structure** for Aqua. Aqua (via its registry) will use templating to locate the right asset from GitHub releases. To be Aqua-compatible, follow these guidelines:

   - **Filename Template:** Use a consistent pattern like `cribo_<version>_<os>_<arch>.<ext>`. We have done this (except `<version>` is implicitly in the release tag context). If you want to include the version in the filename (e.g., `cribo_v1.2.3_linux_amd64.tar.gz`), you can incorporate the tag name into the archive name. This isn’t strictly required for Aqua (the registry can use `{{.Version}}` to insert it), but it can be helpful for clarity. Many Aqua registry entries use the version in asset names. You could modify the packaging step to include the version: for example, use an environment variable or Git context to get the tag. In GitHub Actions, `github.ref_name` for a tag like `v1.2.3` could be used. For instance: `tar -czf cribo_${{ github.ref_name }}_${{ matrix.target_os }}_${{ matrix.target_arch }}.tar.gz cribo`. This would yield `cribo_v1.2.3_linux_amd64.tar.gz`.
   - **Archive Contents:** Each archive should contain the **single executable** (and possibly related files if needed, but typically just the binary). We ensured this by archiving the `cribo` file directly without subfolders. In Aqua’s registry, if the archive had a nested folder, we would need to specify a `files:` mapping to pick the binary. By keeping the binary at the root of the archive, Aqua can install it without extra configuration.
   - **File Extensions:** We are using `.tar.gz` for Linux/Mac and `.zip` for Windows, which Aqua supports. The Aqua registry can handle different formats via the `format` and `format_overrides` fields. Our naming pattern and use of standard extensions (.zip, .tar.gz) align with common practice in Aqua’s standard registry.

   With these conventions, writing the Aqua registry entry (next step) will be straightforward.

6. **Example Aqua Registry YAML Snippet:**\
   To enable Aqua users to install Cribo, we need to add Cribo to Aqua’s **Standard Registry** (the `aquaproj/aqua-registry` repository). Below is an example YAML snippet for the registry entry. This is what we would contribute upstream (or use in a custom registry) once our release assets are in place:

   ```yaml
   - type: github_release
     repo_owner: <GitHub-Owner> # e.g., "my-org" or "my-username"
     repo_name: cribo
     description: 'Cribo - A CLI tool for XYZ (One-line description)'
     asset: 'cribo_{{trimV .Version}}_{{.OS}}_{{.Arch}}.{{.Format}}'
     format: tar.gz
     format_overrides:
       - goos: windows
         format: zip
     replacements:
       darwin: Darwin
       linux: Linux
       windows: Windows
       amd64: x86_64
       arm64: arm64
   ```

   Let’s interpret this snippet:
   - It specifies that Cribo is distributed via GitHub releases (`type: github_release`) from a given repository owner/name.
   - `asset` defines the naming pattern of the release files. Aqua will replace `{{.Version}}`, `{{.OS}}`, and `{{.Arch}}` with the requested version and the user’s platform info. We use `trimV` on Version to drop the leading "v" (if our Git tags are like v1.2.3). The pattern must match our actual filenames. Given our setup, if we included the version in filenames, this pattern is correct. If not including version in filenames, the pattern could be adjusted (e.g., `cribo_{{.OS}}_{{.Arch}}.{{.Format}}`).
   - `format: tar.gz` and the Windows override indicate that by default the archives are tarballs, except on Windows where they are zips. This matches what we implemented.
   - `replacements` map the OS/arch strings Aqua uses to those in our filenames. For example, Aqua might use `darwin` internally for Mac, but our file names use `Darwin` (capital D) or vice versa. In our case, we lowercased the OS in the file name (e.g., "linux"), but many projects capitalize them in assets. We include these replacements to be safe. Notably, we map `amd64` to `x86_64` because many release assets use “x86_64” to denote 64-bit Intel/AMD architecture. If our archives use `amd64` literally, we could instead map `amd64: amd64` or omit that line. The above mapping is aligned with typical conventions in the Aqua registry (for example, the entry for **Switchboard** maps Darwin/Linux/Windows and x86_64). Adjust these mappings if your actual filenames differ.

   **After adding this entry** to the Aqua registry (via a pull request to `aquaproj/aqua-registry`), Aqua users will be able to install Cribo by referencing the package name (e.g., `name: <GitHub-Owner>/cribo@<version>` in their `aqua.yaml`). Aqua will fetch the correct archive and extract the `cribo` binary for the user.

7. **UBI Compatibility and Documentation Updates:**\
   With multi-platform release assets in place, Cribo is now compatible with **UBI (Universal Binary Installer)** out-of-the-box. UBI is a CLI tool that automatically downloads the appropriate binary from GitHub releases given a GitHub `owner/repo` and optionally a version. Our consistent naming scheme ensures UBI can identify the right file. Specifically, UBI will parse the list of release assets and look for one that matches the current OS and architecture (it uses regex patterns to match OS names like "linux", "darwin", "windows" and arch like "x86_64", "arm64", etc.). By including those strings in our filenames, we allow UBI’s logic to quickly zero in on the correct archive.

   We should update Cribo’s documentation (e.g., README) to inform users of the new installation methods. Below is a snippet that can be added to the documentation:

   ````markdown
   ### Installing Cribo via Aqua

   If you use [aquaproj/aqua](https://aquaproj.github.io/), you can install Cribo by adding it to your Aqua registry config. For example, after Cribo is added to Aqua’s Standard Registry, include in your `aqua.yaml`:

   ```yaml
   registries:
     - type: standard
       ref: latest # uses the latest standard registry
   packages:
     - name: <GitHub-Owner>/cribo@<version>
   ```
   ````
   Then run `aqua install` to fetch the Cribo CLI. Aqua will download the appropriate binary for your platform and place it in your `~/aqua/bin`.

   ### Installing Cribo via UBI

   You can also use the [Universal Binary Installer (UBI)](https://github.com/houseabsolute/ubi) to get Cribo. First, install the `ubi` tool (e.g., via Homebrew: `brew install ubi`, or the bootstrap script from UBI’s repo). Then run:
   ```bash
   ubi -p <GitHub-Owner>/cribo
   ```
   This will download the latest Cribo binary for your OS and install it (by default, into `./bin` or you can specify `-i /usr/local/bin`). If you want a specific version, append the `-t` flag with the tag, for example:
   ```bash
   ubi --project <GitHub-Owner>/cribo --tag v1.2.3
   ```
   (The `--project` (or `-p`) flag specifies the GitHub repo, and `--tag` (or `-t`) selects the release tag. If no tag is provided, UBI grabs the latest release.) After running this, you should be able to use the `cribo` command directly.
   ```
   In the above docs snippet, replace `<GitHub-Owner>` with the actual GitHub username or organization name of the Cribo repository, and use a real version number in place of `<version>` or `v1.2.3` as needed. This documentation guides users through Aqua and UBI installation methods, which are now possible thanks to our updated release workflow.
   ```

## Conclusion

By implementing the changes outlined above, the Cribo project’s release process will produce cross-platform CLI binaries, checksums, and attach them to GitHub Releases. These modifications not only broaden Cribo’s accessibility (users can download a binary for their platform directly) but also integrate with popular installer tools **Aqua** and **UBI**. Aqua users will benefit from a one-line configuration to manage Cribo’s version, and UBI users can install or update Cribo with a single command.

All the steps are designed to be self-contained in the `release.yml` workflow. Once updated, verify the next release by checking that the assets (e.g., `.tar.gz` and `.zip` files for each OS, plus `.sha256` files) appear on the GitHub Releases page. It’s also wise to test Aqua and UBI installations manually to ensure everything works as expected. With this setup, Cribo’s distribution is more user-friendly and aligned with best practices for CLI tool releases, providing a smooth experience for developers using the tool.

**Sources:**

- Eugene Babichenko, *Automated multi-platform releases with GitHub Actions* – discusses using matrix builds and uploading assets in release workflows.
- Aqua Registry (Standard) – example entries for CLI tools (e.g., Switchboard) showing asset naming conventions and YAML format.
- UBI (Universal Binary Installer) Documentation – explains how UBI selects the correct release asset based on OS/arch substrings in filenames and usage of the `ubi` CLI.
