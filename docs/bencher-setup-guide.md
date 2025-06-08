# Bencher.dev Integration Setup Guide for Cribo

This guide provides step-by-step instructions for setting up Bencher.dev API token integration in the Cribo GitHub repository to enable continuous benchmarking.

## Prerequisites

- Admin access to the `ophidiarium/cribo` GitHub repository
- A Bencher.dev account (free tier available at https://bencher.dev)
- The Cribo project created in Bencher.dev

## Step 1: Create a Bencher.dev Account and Project

1. **Sign up for Bencher.dev**
   - Go to https://bencher.dev
   - Click "Sign Up" and create an account
   - Verify your email address

2. **Create the Cribo Project**
   - Log in to your Bencher.dev dashboard
   - Click "New Project" or "Create Project"
   - Enter the following details:
     - **Project Name**: `cribo` (MUST match the `BENCHER_PROJECT` in workflow)
     - **Project Slug**: `cribo` (auto-generated, verify it matches)
     - **Description**: "Python source bundler written in Rust"
     - **Visibility**: Choose based on your preference (public recommended for open source)
   - Click "Create Project"

## Step 2: Generate a Bencher.dev API Token

1. **Navigate to API Tokens**
   - In your Bencher.dev dashboard, click on your profile icon
   - Select "API Tokens" or go to https://bencher.dev/console/settings/tokens

2. **Create a New Token**
   - Click "New Token" or "Generate Token"
   - Enter token details:
     - **Name**: `GitHub Actions - Cribo`
     - **Permissions**: Select "Write" permissions for the `cribo` project
     - **Expiration**: Set to "Never" or a long duration
   - Click "Generate Token"

3. **Copy the Token**
   - **IMPORTANT**: Copy the generated token immediately
   - You won't be able to see it again after leaving this page
   - Store it securely until you add it to GitHub

## Step 3: Add the API Token to GitHub Repository Secrets

1. **Navigate to Repository Settings**
   - Go to https://github.com/ophidiarium/cribo
   - Click on "Settings" tab (requires admin access)

2. **Access Secrets and Variables**
   - In the left sidebar, expand "Secrets and variables"
   - Click on "Actions"

3. **Add the Repository Secret**
   - Click "New repository secret" button
   - Enter the following:
     - **Name**: `BENCHER_API_TOKEN` (MUST match exactly)
     - **Secret**: Paste the API token you copied from Bencher.dev
   - Click "Add secret"

## Step 4: Configure Bencher.dev Project Settings

1. **Access Project Settings**
   - In the Bencher.dev dashboard, navigate to your `cribo` project
   - Click on "Settings" or the gear icon

2. **Configure Testbeds**
   - Go to "Testbeds" section
   - Ensure you have a testbed named `ubuntu-latest` (or create it)
   - This should match the `BENCHER_TESTBED` in the workflow

3. **Set Up Branches** (Optional but recommended)
   - Go to "Branches" section
   - Create a branch for `main`:
     - **Name**: `main`
     - **Start Point**: Set as needed
   - This helps track performance over time

4. **Configure Thresholds** (Optional)
   - Go to "Thresholds" section
   - Set up performance regression thresholds:
     - **Metric**: Choose the metrics you want to monitor
     - **Threshold**: Set percentage or absolute values
     - **Window**: Set the comparison window (e.g., last 10 runs)

## Step 5: Verify the Integration

1. **Trigger a Workflow Run**
   - Make a small change to any file in the repository
   - Create a pull request or push to main branch
   - This will trigger the benchmarks workflow

2. **Check GitHub Actions**
   - Go to https://github.com/ophidiarium/cribo/actions
   - Look for "Continuous Benchmarking" workflow
   - Click on the latest run
   - Verify both jobs complete successfully:
     - "Run Benchmarks with Bencher"
     - "CLI Performance Benchmarks"

3. **Verify in Bencher.dev Dashboard**
   - Go to your Bencher.dev dashboard
   - Navigate to the `cribo` project
   - Check "Reports" or "Perf" section
   - You should see new benchmark data points

4. **Check Pull Request Comments** (for PRs)
   - If you created a PR, Bencher should add a comment
   - The comment will show performance comparisons
   - This requires the `pull-requests: write` permission (already configured)

## Troubleshooting

### Common Issues

1. **"BENCHER_API_TOKEN is not set" Error**
   - Verify the secret name is exactly `BENCHER_API_TOKEN`
   - Check that the secret was saved successfully
   - Try re-creating the secret

2. **"Project not found" Error**
   - Ensure the project name in Bencher.dev exactly matches `cribo`
   - Check the project slug is also `cribo`
   - Verify the API token has permissions for this project

3. **"Unauthorized" Error**
   - Token may have expired
   - Token may not have write permissions
   - Generate a new token and update the secret

4. **No PR Comments Appearing**
   - Verify `pull-requests: write` permission in workflow
   - Check Bencher.dev project settings for PR integration
   - Ensure the workflow is running on PR events

### Debugging Steps

1. **Enable Debug Logging**
   - Add `BENCHER_DEBUG: true` to the env section in workflow
   - This provides detailed output in GitHub Actions logs

2. **Check Raw Benchmark Output**
   - Download artifacts from workflow run
   - Examine `benchmark_results.json` and `cli_results.json`
   - Verify JSON format is correct

3. **Test API Token Locally**
   ```bash
   # Install Bencher CLI
   curl --proto '=https' --tlsv1.2 -sSfL https://bencher.dev/download/install.sh | sh

   # Test token (replace YOUR_TOKEN)
   export BENCHER_API_TOKEN="YOUR_TOKEN"
   bencher project list
   ```

## Additional Configuration Options

### Customize Benchmark Adapters

The workflow uses `json` adapter by default. Other options:

- `cargo_bench` - For standard Cargo bench output
- `criterion` - For Criterion.rs benchmarks
- `hyperfine` - For hyperfine CLI benchmarks

### Add More Testbeds

To benchmark on multiple platforms:

1. Add more jobs with different `runs-on` values
2. Create corresponding testbeds in Bencher.dev
3. Update `BENCHER_TESTBED` environment variable

### Set Up Alerts

1. In Bencher.dev project settings, go to "Alerts"
2. Configure email or webhook notifications for:
   - Performance regressions
   - Threshold violations
   - Failed benchmark runs

## Security Best Practices

1. **Token Rotation**
   - Rotate API tokens periodically
   - Set expiration dates when possible
   - Delete old tokens after rotation

2. **Minimal Permissions**
   - Only grant write access to specific projects
   - Use separate tokens for different purposes

3. **Secret Scanning**
   - Enable GitHub secret scanning
   - Never commit tokens to repository

## Resources

- [Bencher.dev Documentation](https://bencher.dev/docs)
- [Bencher GitHub Action](https://github.com/bencherdev/bencher)
- [Serpen Benchmarks Workflow](.github/workflows/benchmarks.yml)
- [Bencher.dev API Reference](https://bencher.dev/docs/api)

## Support

- **Bencher.dev Issues**: https://github.com/bencherdev/bencher/issues
- **Cribo Issues**: https://github.com/ophidiarium/cribo/issues
- **Bencher Discord**: https://discord.gg/bencherdev
