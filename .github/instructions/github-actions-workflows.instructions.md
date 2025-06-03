---
applyTo: "**/.github/workflows/*.yml,**/github-actions.md"
---

# GitHub Actions Workflow Guidelines

## Overview

This document outlines standards and best practices for implementing Continuous Integration (CI) and Continuous Deployment (CD) workflows using GitHub Actions. Following these guidelines ensures reliable, secure, and efficient deployment pipelines on the GitHub platform.

## GitHub Actions Basics

- **Workflow Files**: Stored in `.github/workflows/*.yml`
- **Events**: Triggers such as push, pull_request, schedule
- **Jobs**: Collections of steps that run on the same runner
- **Steps**: Individual tasks that run commands or actions
- **Actions**: Reusable units of code that can be shared

### Platform Support and Runners

**IMPORTANT**: Always verify current platform support before configuring matrix builds or target platforms. GitHub frequently updates their hosted runners, adds new platforms, and deprecates old ones.

**Required Research Step**: Before implementing or updating any workflow that uses platform matrices, cross-compilation, or specific runner configurations, consult the official GitHub documentation:

📖 **Primary Source**: [GitHub-hosted runners documentation](https://docs.github.com/en/actions/using-github-hosted-runners/using-github-hosted-runners/about-github-hosted-runners#standard-github-hosted-runners-for-public-repositories)

**What to verify**:

- Currently supported operating systems (Ubuntu, Windows, macOS versions)
- Available architectures (x64, ARM64 availability and preview status)
- New additions, including preview/beta runner labels
- Deprecated or sunset runner images
- Hardware specifications and limitations
- Any platform-specific constraints or requirements
- Differences between public and private repository runners

**Implementation Impact**:

- Use only documented runner labels in `runs-on` fields (including preview runners like `ubuntu-24.04-arm`, `windows-11-arm`)
- Configure cross-compilation only for supported target combinations
- Account for preview/beta status when planning production workflows
- Consider hardware limitations when designing build matrices

## Standard Workflow Components

### 1. Continuous Integration

#### Code Quality Checks

```yaml
# Example GitHub Actions workflow for code quality
name: Code Quality

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main, develop]

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Set up environment
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Run linter
        run: npm run lint

  format:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Set up environment
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Check formatting
        run: npm run format:check
```

#### Testing

```yaml
# Example GitHub Actions workflow for testing
name: Tests

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main, develop]

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Set up environment
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Run unit tests
        run: npm run test:unit
      - name: Upload coverage
        uses: codecov/codecov-action@v3
        with:
          token: ${{ secrets.CODECOV_TOKEN }}

  integration-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Set up environment
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Run integration tests
        run: npm run test:integration
```

#### Security Scanning

```yaml
# Example GitHub Actions workflow for security scanning
name: Security

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main, develop]
  schedule:
    - cron: '0 0 * * 0' # Weekly

jobs:
  dependency-scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Set up environment
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Run dependency audit
        run: npm audit

  code-scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Run CodeQL Analysis
        uses: github/codeql-action/analyze@v2
        with:
          languages: javascript
```

### 2. Continuous Deployment

#### Build and Artifact Creation

```yaml
# Example GitHub Actions workflow for building and creating artifacts
name: Build

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main, develop]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Set up environment
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Build
        run: npm run build
      - name: Upload build artifact
        uses: actions/upload-artifact@v3
        with:
          name: build-output
          path: build/
```

#### Deployment

```yaml
# Example GitHub Actions workflow for deployment
name: Deploy

on:
  push:
    branches: [main]

jobs:
  deploy-staging:
    runs-on: ubuntu-latest
    environment: staging
    steps:
      - uses: actions/checkout@v3
      - name: Set up environment
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Build
        run: npm run build
      - name: Deploy to staging
        uses: some-deployment-action@v1
        with:
          api-key: ${{ secrets.DEPLOY_API_KEY }}
          environment: staging

  deploy-production:
    needs: deploy-staging
    runs-on: ubuntu-latest
    environment: production
    steps:
      - uses: actions/checkout@v3
      - name: Set up environment
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Build
        run: npm run build
      - name: Deploy to production
        uses: some-deployment-action@v1
        with:
          api-key: ${{ secrets.DEPLOY_API_KEY }}
          environment: production
```

## GitHub-Specific Features

### Environments and Secrets

- Create environments (Settings → Environments) for different deployment targets
- Configure environment-specific secrets and variables
- Set up required reviewers for production deployments
- Define environment-specific deployment rules

```yaml
jobs:
  deploy:
    environment:
      name: production
      url: https://example.com
    # Job configuration
```

### Branch Protection Rules

- Enable branch protection for main/production branches
- Require status checks to pass before merging
- Require pull request reviews
- Restrict who can push to matching branches

### GitHub Actions Optimizations

#### Caching Dependencies

```yaml
steps:
  - uses: actions/checkout@v3
  - name: Set up environment
    uses: actions/setup-node@v3
    with:
      node-version: '16'
  - name: Cache dependencies
    uses: actions/cache@v3
    with:
      path: ~/.npm
      key: ${{ runner.os }}-node-${{ hashFiles('**/package-lock.json') }}
      restore-keys: |
        ${{ runner.os }}-node-
  - name: Install dependencies
    run: npm ci
```

#### Matrix Builds

```yaml
jobs:
  test:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
        node-version: [14, 16, 18]
    steps:
      - uses: actions/checkout@v3
      - name: Use Node.js ${{ matrix.node-version }}
        uses: actions/setup-node@v3
        with:
          node-version: ${{ matrix.node-version }}
      - name: Install dependencies
        run: npm ci
      - name: Run tests
        run: npm test
```

#### Reusable Workflows

```yaml
# .github/workflows/reusable-build.yml
name: Reusable Build Workflow
on:
  workflow_call:
    inputs:
      node-version:
        required: true
        type: string
    secrets:
      npm-token:
        required: true

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions/setup-node@v3
        with:
          node-version: ${{ inputs.node-version }}
          registry-url: 'https://registry.npmjs.org'
      - name: Install dependencies
        run: npm ci
        env:
          NODE_AUTH_TOKEN: ${{ secrets.npm-token }}
      - name: Build
        run: npm run build
```

Usage in another workflow:

```yaml
jobs:
  call-build:
    uses: ./.github/workflows/reusable-build.yml
    with:
      node-version: '16'
    secrets:
      npm-token: ${{ secrets.NPM_TOKEN }}
```

### GitHub Actions Limitations and Solutions

#### Timeouts and Job Duration

- Default timeout: 6 hours
- Strategies for long-running jobs:
  - Break into smaller jobs
  - Use self-hosted runners
  - Implement appropriate timeouts

```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    timeout-minutes: 30 # Custom timeout
```

#### Storage and Artifacts

- Free plan: 500 MB storage, 1 GB for artifacts
- Strategies to manage storage:
  - Limit artifact retention
  - Compress artifacts before upload
  - Clean up old artifacts

```yaml
- name: Upload artifact
  uses: actions/upload-artifact@v3
  with:
    name: build
    path: build/
    retention-days: 5 # Custom retention period
```

## YAML Quality and Validation

### yamllint Integration

**MANDATORY**: All GitHub Actions workflow files must pass yamllint validation before commit. This ensures consistent formatting, catches syntax errors early, and maintains code quality standards.

#### Configuration

The project uses a custom `.yamllint` configuration optimized for GitHub Actions workflows:

- **Line length**: 120 characters (accommodating GitHub Actions URLs and expressions)
- **Document start**: Optional (GitHub Actions don't require `---`)
- **Truthy values**: Allows common GitHub Actions values like `'on'`, `'off'`
- **Comments**: Flexible spacing requirements

#### Validation Process

**After making any changes to workflow files**:

1. **Run yamllint validation**:
   ```bash
   yamllint .github/workflows/filename.yml
   ```

2. **Fix any reported issues** before committing

3. **Pre-commit hook**: yamllint runs automatically via Lefthook pre-commit hooks

#### Common yamllint Fixes

**Long lines**: Split using YAML continuation or folding:

```yaml
# Before (too long)
description: 'Very long description that exceeds the line length limit and needs to be split'

# After (using folding)
description: >-
  Very long description that exceeds the line length limit
  and needs to be split
```

**Complex shell commands**: Use multi-line format:

```yaml
# Before (too long)
run: cargo test --workspace --all-targets --no-fail-fast -- --format=json -Z unstable-options | cargo2junit

# After (multi-line)
run: |
  cargo test --workspace --all-targets --no-fail-fast -- \
    --format=json -Z unstable-options | cargo2junit
```

#### VS Code Integration

When using VS Code, yamllint errors are automatically highlighted. The custom configuration ensures realistic validation for GitHub Actions workflows while maintaining code quality.

### YAML Syntax and Script Integration Best Practices

#### ⚠️ Critical Warning: Python Inline Scripts in YAML

**AVOID using multi-line Python scripts directly in YAML workflow files**. This approach commonly leads to:

- **YAML indentation errors** that break workflow parsing
- **Syntax errors** from Python code conflicting with YAML structure
- **Hard-to-debug failures** where YAML parser errors obscure the actual issue
- **Maintenance complexity** mixing Python and YAML syntax

**❌ Problematic Pattern:**

```yaml
- name: Complex Python operation
  shell: python
  run: |
    import zipfile
    import os
    wheel_file = "${{ env.WHEEL_FILE }}"
    with zipfile.ZipFile(wheel_file, 'r') as zip_ref:
        for file_info in zip_ref.infolist():
            if file_info.filename.endswith('.dist-info/WHEEL'):
                content = zip_ref.read(file_info).decode('utf-8')
                print(content)
```

**✅ Recommended Alternatives:**

1. **Use shell commands with appropriate tools:**
   ```yaml
   - name: Inspect wheel contents
     run: unzip -p "${{ env.WHEEL_FILE }}" "*.dist-info/WHEEL"
   ```

2. **Extract to separate Python script file:**
   ```yaml
   - name: Run Python analysis
     run: python scripts/analyze_wheel.py "${{ env.WHEEL_FILE }}"
   ```

3. **Use existing GitHub Actions from marketplace:**
   ```yaml
   - name: Analyze Python package
     uses: some-org/wheel-analyzer-action@v1
     with:
       wheel-path: ${{ env.WHEEL_FILE }}
   ```

#### Shell Script Best Practices

**Keep inline shell scripts simple and focused:**

```yaml
# ✅ Good: Simple, single-purpose commands
- name: Extract version
  run: echo "VERSION=$(grep version pyproject.toml | cut -d '"' -f 2)" >> $GITHUB_ENV

# ✅ Good: Multi-line for readability, but still simple
- name: Build and test
  run: |
    cargo build --release
    cargo test --workspace
    echo "Build completed successfully"

# ❌ Avoid: Complex logic mixing multiple languages/tools
- name: Complex analysis
  run: |
    python -c "
    import sys, json, subprocess
    result = subprocess.run(['cargo', 'metadata'], capture_output=True, text=True)
    data = json.loads(result.stdout)
    # ... 20 more lines of Python ...
    "
```

#### YAML Syntax Validation Checklist

Before committing workflow changes:

1. **Run yamllint validation**: `yamllint .github/workflows/`
2. **Check indentation consistency**: Use 2-space indentation throughout
3. **Validate multi-line strings**: Use `|` or `>` operators correctly
4. **Test locally if possible**: Use tools like `act` for local testing
5. **Review in GitHub UI**: Check that syntax highlighting looks correct

#### Common YAML Pitfalls in Workflows

**Inconsistent indentation:**

```yaml
# ❌ Wrong
steps:
  - name: Step 1
    run: echo "hello"
    - name: Step 2  # Wrong indentation
      run: echo "world"

# ✅ Correct
steps:
  - name: Step 1
    run: echo "hello"
  - name: Step 2
    run: echo "world"
```

**Incorrect multi-line string handling:**

```yaml
# ❌ Wrong: Will cause parsing errors
- name: Multi-line command
  run: echo "This is a very long command that spans multiple lines
    and will break YAML parsing"

# ✅ Correct: Use proper YAML multi-line syntax
- name: Multi-line command
  run: |
    echo "This is a very long command that spans multiple lines"
    echo "and is properly formatted"
```

**Environment variable syntax errors:**

```yaml
# ❌ Wrong: Missing quotes can cause parsing issues
- name: Use environment variable
  run: echo ${{ env.MY_VAR }}

# ✅ Correct: Properly quoted
- name: Use environment variable
  run: echo "${{ env.MY_VAR }}"
```

## Security Best Practices

- Limit permissions for GitHub tokens
- Pin action versions with SHA references
- Avoid printing secrets to logs
- Validate inputs for scripts and commands
- Use OpenID Connect for cloud provider authentication

```yaml
jobs:
  deploy:
    # Limit token permissions
    permissions:
      contents: read
      deployments: write
    runs-on: ubuntu-latest
    steps:
      # Pin action versions with SHA
      - uses: actions/checkout@8e5e7e5ab8b370d6c329ec480221332ada57f0ab # v3.5.2
```

## GitHub Actions Monitoring

- Use status badges on README
- Set up notifications for workflow failures
- Monitor workflow usage and costs
- Configure billing alerts
- Review workflow analytics

## GitHub Actions CI/CD Patterns

### Monorepo Pattern

```yaml
name: Monorepo CI

on:
  push:
    paths:
      - 'packages/frontend/**'
      - 'packages/backend/**'
      - 'packages/common/**'

jobs:
  detect-changes:
    runs-on: ubuntu-latest
    outputs:
      frontend: ${{ steps.filter.outputs.frontend }}
      backend: ${{ steps.filter.outputs.backend }}
    steps:
      - uses: actions/checkout@v3
      - uses: dorny/paths-filter@v2
        id: filter
        with:
          filters: |
            frontend:
              - 'packages/frontend/**'
              - 'packages/common/**'
            backend:
              - 'packages/backend/**'
              - 'packages/common/**'

  frontend:
    needs: detect-changes
    if: ${{ needs.detect-changes.outputs.frontend == 'true' }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Setup Node.js
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: cd packages/frontend && npm ci
      - name: Run tests
        run: cd packages/frontend && npm test
```

### Feature Branch Pattern

```yaml
name: Feature Branch CI

on:
  push:
    branches:
      - 'feature/**'
      - 'bugfix/**'

jobs:
  build-and-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Setup Node.js
        uses: actions/setup-node@v3
        with:
          node-version: '16'
      - name: Install dependencies
        run: npm ci
      - name: Run tests
        run: npm test
      - name: Create preview environment
        id: preview
        run: |
          BRANCH_NAME=${GITHUB_REF##*/}
          echo "Creating preview for branch $BRANCH_NAME"
          PREVIEW_URL="https://preview-${BRANCH_NAME}.example.com"
          echo "url=$PREVIEW_URL" >> $GITHUB_OUTPUT
      - name: Comment PR with preview URL
        uses: actions/github-script@v6
        with:
          script: |
            const previewUrl = '${{ steps.preview.outputs.url }}';
            const issueNumber = context.issue.number;
            github.rest.issues.createComment({
              issue_number: issueNumber,
              owner: context.repo.owner,
              repo: context.repo.repo,
              body: `Preview environment deployed to: ${previewUrl}`
            });
```

## Migration from Other CI/CD Systems

### From Jenkins to GitHub Actions

| Jenkins          | GitHub Actions                           |
| ---------------- | ---------------------------------------- |
| Jenkinsfile      | workflow YAML files                      |
| stages           | jobs                                     |
| steps            | steps                                    |
| agents           | runners                                  |
| shared libraries | reusable workflows and composite actions |
| credentials      | secrets                                  |

### From CircleCI to GitHub Actions

| CircleCI  | GitHub Actions           |
| --------- | ------------------------ |
| orbs      | actions                  |
| workflows | workflow files           |
| jobs      | jobs                     |
| executors | runners                  |
| commands  | composite actions        |
| contexts  | environments and secrets |

## Common Pitfalls and Gotchas

### Frequent Mistakes Developers Make

- **Incorrect Syntax:** YAML syntax can be tricky. Use a linter or validator to catch syntax errors.
- **Incorrect Indentation:** Indentation is crucial in YAML. Use consistent indentation throughout your workflow files.
- **Missing Permissions:** Grant workflows the necessary permissions to access resources (e.g., repository contents, secrets).
- **Typos in Secrets:** Double-check the names of your secrets to avoid typos.
- **Not Pinning Action Versions:** Always pin actions to specific versions or commits to prevent unexpected changes.

### Edge Cases to Be Aware Of

- **Rate Limits:** Be aware of GitHub API rate limits. Implement retry logic to handle rate limit errors.
- **Concurrent Workflow Runs:** Handle concurrent workflow runs gracefully to avoid conflicts.
- **Network Issues:** Implement error handling to handle network issues and transient errors.
- **Large File Sizes:** Be aware of the maximum file sizes supported by GitHub Actions.

### Version-Specific Issues

- **Action Compatibility:** Ensure that your actions are compatible with the version of GitHub Actions you are using.
- **Runner Images:** Be aware of the changes in runner images and update your workflows accordingly.

### Compatibility Concerns

- **Cross-Platform Compatibility:** Ensure that your workflows are compatible with different operating systems (e.g., Linux, Windows, macOS).
- **Browser Compatibility:** If your workflows involve web applications, test them in different browsers.

### Debugging Strategies

- **Workflow Logs:** Examine workflow logs to identify errors and warnings.
- **Debugging Actions:** Use debugging actions to inspect the state of your workflows.
- **Step-by-Step Debugging:** Insert `echo` statements or debugging actions to trace the execution of your workflows step by step.
- **Local Testing:** Use tools like `act` to test your workflows locally before pushing them to GitHub.

## Tooling and Environment

### Recommended Development Tools

- **VS Code with GitHub Actions Extension:** Use VS Code with the GitHub Actions extension for syntax highlighting, code completion, and validation.
- **GitHub CLI:** Use the GitHub CLI to interact with the GitHub API from your workflows.
- **`act`:** Use `act` to test your workflows locally.
- **YAML Linter:** Use a YAML linter to catch syntax errors in your workflow files.

### Build Configuration

- **`.github/workflows/`:** Place all workflow files in this directory.
- **`action.yml`:** For reusable actions, define their metadata in this file.

### Linting and Formatting

- **YAML Lint:** Use a YAML linting tool to enforce consistent formatting and catch syntax errors.
- **Shellcheck:** Use Shellcheck to lint your shell scripts.
- **Prettier:** Use Prettier to format your JavaScript and CSS files.

### Deployment Best Practices

- **Environment Variables:** Use environment variables to configure your deployments.
- **Deployment Strategies:** Use appropriate deployment strategies (e.g., blue/green deployment, canary deployment) to minimize downtime.
- **Rollback Strategies:** Implement rollback strategies to revert to a previous version if a deployment fails.

### CI/CD Integration

- **Continuous Integration (CI):** Run automated tests on every commit to ensure code quality.
- **Continuous Delivery (CD):** Automate the deployment process to deliver new features and bug fixes to users quickly.
- **Automated Releases:** Automate the release process to create and publish releases automatically.

## Resources

- [GitHub Actions Documentation](https://docs.github.com/en/actions)
- [GitHub Actions Marketplace](https://github.com/marketplace?type=actions)
- [GitHub Actions Community Forum](https://github.community/c/actions/41)
- [GitHub Actions Starter Workflows](https://github.com/actions/starter-workflows)
