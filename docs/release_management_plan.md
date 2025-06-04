# Automated Release Management for a Rust Project with Release Please

In this guide, we integrate **Google’s release-please** into a Rust GitHub project to automate changelog generation, semantic version bumping, and GitHub releases. We will use **GitHub Actions** (already in use in the project) and enforce the **Conventional Commits** standard for commit messages. This approach is ideal for a solo developer (with AI assistants) because it minimizes manual release tasks. We’ll cover step-by-step how to set up release-please, configure it for Rust, enforce commit message conventions with Lefthook and CI checks, and provide guidelines for AI-driven contributions.

## 1. Adopt the Conventional Commits Standard

To enable automated versioning, **all commit messages should follow the Conventional Commits** format. This convention makes commit history machine-readable for determining release notes and version bumps. The basic format is:

```
<type>(<optional scope>): <description>
```

- **Types** (common examples: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `ci`) indicate the nature of the change.
- **Scope** is optional (e.g. a module or component name).
- **Description** is a short summary of the change.

**Breaking changes** are indicated by an exclamation mark (e.g. `feat!:`) or a `BREAKING CHANGE:` footer. According to release-please and SemVer rules:

- A `fix:` commit triggers a **patch** version bump (e.g. 1.0.0 → 1.0.1).
- A `feat:` commit triggers a **minor** version bump (e.g. 1.0.0 → 1.1.0).
- A commit with **`!`** or a "BREAKING CHANGE" triggers a **major** version bump (e.g. 1.0.0 → 2.0.0).

By adopting this standard, release-please can automatically determine the next version and generate changelog entries. Ensure that you and any AI agents writing code use this format for every commit. For example:

- `feat(parser): add support for new syntax`
- `fix: handle null pointer exception in module X`
- `chore: update dependencies`

## 2. Set Up Lefthook for Commit Message Linting

To enforce the commit message format, use **Lefthook** (a Git hooks manager) to run a commit-msg hook. This will lint commit messages **before they are saved**, preventing invalid messages from ever entering the repository.

**Steps:**

1. **Install Lefthook:** You can install it as a standalone binary or via a package manager. For example, as an NPM dev dependency:
   ```bash
   npm install --save-dev @evilmartians/lefthook
   ```
   Then run:
   ```bash
   npx lefthook install
   ```
   (Alternatively, install via Homebrew, Cargo, etc., depending on your environment.)

2. **Install Commitlint:** Commitlint is a widely-used tool to validate Conventional Commit messages. Install it in the project:
   ```bash
   npm install --save-dev @commitlint/cli @commitlint/config-conventional
   ```
   This will add a `commitlint` binary and config to your project.

3. **Add a `commitlint.config.js` file:** In the repository root, create `commitlint.config.js` with the following content to use the standard configuration:
   ```js
   // commitlint.config.js
   module.exports = { 
     extends: ['@commitlint/config-conventional'] 
   };
   ```

4. **Configure Lefthook’s hook:** Add a `lefthook.yml` file (or edit it if it exists) at the project root with a `commit-msg` hook that runs commitlint. For example:
   ```yaml
   commit-msg:
     commands:
       'lint commit message':
         run: npx commitlint --edit {1}
   ```
   This uses Commitlint to lint the commit message file passed as `{1}` (the first argument, which Lefthook sets to the commit message file path). If a commit message doesn’t meet the Conventional Commits format, this hook will reject the commit with an error message.

5. (Optional) **Interactive commit messages:** You can also set up a `prepare-commit-msg` hook with tools like **Commitizen** to interactively prompt for a well-formatted commit message. For example, add Commitizen (`commitizen` and `cz-conventional-changelog` packages) and configure Lefthook to run `npx cz` on commit. This is optional – the key requirement is that commits follow the conventional format.

After this setup, any commit done locally will be validated. This prevents bad commit messages upfront. Keep in mind that local hooks can be bypassed (intentionally or if an AI commits changes directly), so we’ll also add a server-side check in CI.

## 3. Add GitHub Action to Enforce Conventional Commits on PRs

To catch any commit messages that might slip through local hooks (for example, via GitHub’s UI or automation), set up a **GitHub Actions workflow** to validate commit messages in all pull requests. This ensures that every PR meets the Conventional Commits standard before it’s merged.

A convenient solution is to use a pre-built action like **webiny/action-conventional-commits** which checks all commits in a PR for compliance. For example, create a file `.github/workflows/commitlint.yml` with the following content:

```yaml
name: Conventional Commits Check
on:
  pull_request:
    branches: [main]
jobs:
  commitlint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: webiny/action-conventional-commits@v1.3.0
        with:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          # Optionally, restrict allowed types:
          # allowed-commit-types: "feat,fix,docs,style,refactor,test,chore,ci"
```

This workflow will run for each PR. It checks that **each commit message in the PR** follows the spec, failing the check if any commit is non-conforming. By default, it allows all Conventional Commit types; you can set `allowed-commit-types` if you want to limit the types.

**Branch protection:** You should mark this check as required in your repository’s branch protection settings for the `main` branch. That way, a PR cannot be merged unless the commit message check passes, enforcing the standard at the PR level.

Together, Lefthook (local) and this GitHub Action (remote) ensure commit messages are consistently formatted according to Conventional Commits. This is critical for release-please to automate changelogs and versioning.

## 4. Configure Release Please for a Rust Project

Next, we configure release-please by adding its **manifest and config files** to the repository. These files tell release-please how to handle version bumps and changelog updates for your Rust crate.

### 4.1. Create a `.release-please-manifest.json`

This JSON file is the **source of truth for version numbers**. It tracks the current released version of each package in the repo. Even for a single-crate repository, you need this file so release-please knows the current version (especially if it’s the first release or if it needs to avoid searching tags).

For a single Rust crate at the repository root, use the path `"."` as the key, and set its value to the current version of the crate (from `Cargo.toml`). For example, if your crate is currently version 0.1.0, your `.release-please-manifest.json` should look like:

```json
{
    ".": "0.1.0"
}
```

If you have a Cargo workspace with multiple crates or if you want to use a package name as key, you can list each package path or name here. In our case, using `"."` covers the root crate.

### 4.2. Create a `release-please-config.json`

This file is the **central configuration** for the release-please action. It defines release behavior and can specify multiple packages. For our Rust project, add the following:

````json
{
  "$schema": "https://raw.githubusercontent.com/googleapis/release-please/main/schemas/config.json",
  "packages": {
    ".": {
      "release-type": "rust",
      "changelog-path": "CHANGELOG.md",
      "include-v-in-tag": true,
      "include-component-in-tag": false,
      "draft": false,
      "prerelease": false,
      "bump-minor-pre-major": true,
      "bump-patch-for-minor-pre-major": true
    }
  }
}
Here’s how this config works:
- **`release-type: "rust"`** – Instructs release-please to update `Cargo.toml` versions and handle Rust-specific SemVer behavior.
- **`changelog-path: "CHANGELOG.md"`** – Path to your changelog file. Release-please will append new release notes here on each release.
- **`include-v-in-tag: true`** – Prefixes tags with “v” (e.g., `v1.2.3`) to match common Rust tag conventions.
- **`include-component-in-tag: false`** – Useful in monorepos; set to false for a single-crate repository.
- **`draft: false`** – New GitHub releases are published immediately, not as drafts.
- **`prerelease: false`** – Releases are not marked as prereleases.
- **`bump-minor-pre-major: true`** – When on a 0.x version (e.g., 0.2.0), a breaking change bumps the minor (0.3.0) instead of going to 1.0.0.
- **`bump-patch-for-minor-pre-major: true`** – On 0.x versions, a `feat:` commit bumps the patch (0.2.1) instead of the minor (0.3.0), reserving minor bumps for breaking changes.

Commit these two JSON files to the repository. They direct release-please on how to categorize commits, bump versions, and format tags for your Rust crate.

## 5. Implement the Release-Please GitHub Actions Workflow

With config in place, set up a GitHub Actions workflow to run release-please. This workflow will automate the creation of **Release PRs** and actual releases:

**Release PR Flow:** When changes land on the `main` branch, release-please will aggregate them into a **Release Pull Request** rather than immediately tagging a release. The Release PR will contain:
- An updated `CHANGELOG.md` with entries generated from commit messages.
- An updated `Cargo.toml` with the new version.
- A PR title like `chore(main): release x.y.z` and a description listing the changes (grouped by type).

You can choose to have this run on a schedule (e.g., daily) or on each push to `main`. For minimal human oversight, a daily schedule might bundle changes into one release PR per day. If you prefer immediate releases, run on each push to `main` (every merge triggers it). Here’s a workflow that does both:
```yaml
name: Automated Releases
on:
  push:
    branches: [ main ]
  schedule:
    - cron: '0 0 * * *'   # runs daily at midnight UTC (adjust as needed)

jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write           # needed to create tags and releases
      pull-requests: write      # needed to create/update the release PR
    steps:
      - name: Checkout repo
        uses: actions/checkout@v3

      - name: Run release-please
        id: release
        uses: google-github-actions/release-please-action@v4
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
          config-file: release-please-config.json
          manifest-file: .release-please-manifest.json
````

**Key points in this workflow:**

- We trigger on `push` to `main` *and* on a daily cron schedule. (You can use either or both. Using both ensures if a commit is pushed, it runs soon, and the schedule acts as a fallback to catch any missed events or bundle multiple changes.)
- **Permissions:** We set `contents: write` and `pull-requests: write` at the job level. This is crucial – by default GitHub token has read-only perms for content in workflows triggered by `push`. Release-please needs to **open PRs, push tags, and create releases**, which are write operations. Without these permissions, the action will fail to create the PR or release.
- **Checkout:** We use `actions/checkout@v3` to ensure the repo code is present. While release-please mostly uses GitHub API, it may update files (like `Cargo.toml` and `CHANGELOG.md`) in the context of the PR branch.
- **Release Please action:** We use the official action `google-github-actions/release-please-action@v4`. We provide:
  - `token: ${{ secrets.GITHUB_TOKEN }}` – so it can authenticate to create PRs and releases.
  - `config-file` and `manifest-file` pointing to the files we created in step 4. This tells the action to use our config (release type “rust”, etc.) and to update our manifest. The action will automatically increment the version in `.release-please-manifest.json` when a release is made.

  With `config-file` specified, we do **not** need to explicitly set `release-type` or `package-name` in the workflow YAML; those are defined per package in the config file. The action will detect changes since the last release (based on the manifest’s version or last Git tag) and decide if a release is necessary:
  - If there are no new commits with `feat`/`fix` (or breaking changes) since the last version, it will do nothing (no release needed).
  - If there are conventional commits indicating new changes, it will open or update a Release PR. If a Release PR already exists (open with label `autorelease: pending`), it updates that PR’s content with any new commits. This PR remains open, accumulating changes, until merged.

- **Merging the Release PR:** Once you are ready to cut a release (which could be as soon as the PR opens, or after some accumulation of changes), you merge the Release PR. Upon merge, release-please will:
  1. Commit the changes (updated `Cargo.toml` and `CHANGELOG.md`) to the `main` branch.
  2. Tag the commit with the new version (e.g. `v0.2.0`).
  3. Create a GitHub Release with that tag and the changelog notes.

  You’ll see the PR label change from `autorelease: pending` to `autorelease: tagged` when the tag is pushed, and a new GitHub release will appear (with the changelog). The release-please action automates this entire sequence once the PR is merged.

**Auto-merging the Release PR (optional):** To minimize human overhead further, you can automate the merging of the Release PR. For example:

- Enable [auto-merge](https://docs.github.com/en/pull-requests/configuring-automatic-pull-requests/automatically-merging-a-pull-request) for your repository. You could allow the Release PR to auto-merge when checks pass (you might need to mark the release PR with a label or use GitHub’s auto-merge feature if no review is required).
- Or use a bot/action: some workflows use a GitHub Action (like `peter-evans/enable-pull-request-automerge`) right after creating the Release PR to mark it for auto-merge. If your branch protections allow it (e.g., require the commit message check and perhaps CI tests to pass), the PR will merge itself.

This way, the release becomes completely automated: once a commit is pushed to `main`, the action opens/updates a release PR, and the PR will merge on its own when everything is green, triggering the actual release. Ensure your tests (if any) also run on the release PR or on push, so that a faulty release doesn’t get auto-published. Since you’re a solo dev possibly using AI agents, this no-human loop can save time, but use it with caution.

## 6. Guidelines for AI-Powered Contributions

If AI agents or automated contributors are creating pull requests or commits daily, set expectations so that they integrate smoothly into this release workflow:

- **Follow Conventional Commits in every commit:** AI agents should be instructed to format commit messages properly (the Lefthook and CI checks will enforce this). For example, an AI-generated commit adding a feature should begin with `feat: ...` and a short description of the change. This ensures the change will appear in the changelog and bump the version appropriately. Poorly formatted messages will be rejected by the pipeline.
- **Do not update the version or changelog manually:** The agent should **not** edit `Cargo.toml` version or the `CHANGELOG.md`. Release-please will handle version bumps and changelog entries. Agents can mention changes in PR descriptions if needed for human context, but they shouldn’t modify the changelog file – it will be auto-generated in the Release PR.
- **Pull Request titles and descriptions:** If using squash merges, ensure the **PR title** follows conventional commit format (since the squash commit message defaults to the title). It might be safer to avoid squash merging and merge commits normally, preserving each AI commit (since each commit is already well-formatted). This way, multiple changes can appear separately in the changelog. If you do squash, double-check the final commit message. In either case, instruct the AI that PR titles should be concise and possibly Conventional Commit style (to be safe).
- **Granularity of commits:** Encourage the AI to group changes logically. Each independent change or fix should be a separate commit with an appropriate type. This will produce a clearer changelog. For example, if an AI fixes two different bugs in one day, it could make two commits like `fix: correct off-by-one error in parser` and `fix: prevent crash on null input`. These will appear as two bullet points under the bug fixes section in the release notes.
- **Consistent daily workflow:** If the AI opens daily PRs, you might configure the release-please action to only run on schedule (daily) rather than every push, so that it processes that day’s merged PRs all at once. Communicate to the AI that once its PR is merged to `main`, a release PR will be generated that same day with its changes. The AI doesn’t need to worry about making a changelog entry; it should instead ensure commit messages are descriptive, since those will be used in the changelog.
- **Changelog expectations:** The AI (and you) can expect that every merged `feat` or `fix` will show up in the **CHANGELOG.md** under a new release. The changelog groups changes by type (features, bug fixes, etc.) and by packages if multiple. Non-user-facing commits (chore, docs, refactor, etc.) typically do *not* appear in the changelog for a release unless a breaking change is involved. So the AI’s documentation or refactor commits won’t clutter release notes. It might be useful to inform the AI that only significant commits (features, fixes, perf improvements, and breaking changes) will be noted in release notes.
- **Testing and CI:** Ensure that any continuous integration tests also run for the AI’s PRs and for the release PR. The AI should aim to keep tests passing. This way, when auto-merge of the release PR is enabled, you can trust that a release won’t go out with failing tests. If the AI is generating code, maybe include instructions to update or create tests accordingly (this is outside the scope of release-please, but important for a healthy project).

By following these guidelines, your AI assistants will produce contributions that seamlessly integrate with the automated release process. The human overhead is reduced to near-zero: you can largely trust the pipeline to cut releases daily or as needed.

Finally, it’s wise to **monitor the first few releases manually**. Verify that the version increments, changelog, and published GitHub releases look correct. Once verified, you’ll have a robust, hands-off release system in place, similar to what many open-source projects use. For example, the above setup is modeled on practices from other projects that use release-please for Rust and commit linting with Lefthook – ensuring reliability and consistency.

**Sources:**

- Google Release Please documentation and examples
- Conventional Commits specification and commit message guidelines
- Lefthook and Commitlint integration guide
- Webiny Conventional Commits Action (commit message CI) usage
- Blog posts on automated releases with Conventional Commits
