# NPM Package Provenance

## Overview

Starting with our GitHub Actions releases, all Serpen NPM packages are published with **provenance attestations** that provide cryptographic proof of the package's origin and build process. This enhances supply chain security by enabling verification of package integrity and authenticity.

## What is NPM Provenance?

NPM provenance uses the [SLSA (Supply-chain Levels for Software Artifacts)](https://slsa.dev/) framework to create verifiable statements about how packages were built. Each package includes:

- 🔐 **Cryptographic Signatures**: Packages are signed using [Sigstore](https://sigstore.dev/) certificates
- 📋 **Build Attestations**: Links to the exact source code, commit, and build process
- 🛡️ **Tamper Protection**: Detects if packages have been modified after publication
- 🏷️ **Verification Badges**: Visual indicators on npmjs.com showing provenance status

## Verifying Serpen Packages

### Using npm audit signatures

You can verify the provenance of any Serpen package using npm's built-in verification:

```bash
# Verify a specific package
npm audit signatures serpen

# Verify all packages in your project
npm audit signatures
```

### Expected Output

When verification succeeds, you'll see output like:

```
audited 1 package in 0.5s

1 package has a verified registry signature
```

### Provenance Information

To view detailed provenance information:

```bash
# Install jq for JSON parsing (if not already installed)
npm install -g jq

# View provenance details
npm view serpen --json | jq '.dist.attestations'
```

This will show:

- **Source repository**: GitHub URL where the package was built
- **Commit SHA**: Exact commit used to build the package
- **Workflow**: GitHub Actions workflow that performed the build
- **Build environment**: Details about the CI/CD environment

## Security Benefits

### Supply Chain Protection

1. **Source Verification**: Confirms packages were built from the official Serpen repository
2. **Build Integrity**: Ensures packages weren't tampered with during the build process
3. **Reproducible Builds**: Links packages to specific commits and build instructions
4. **Certificate Transparency**: Uses public certificate logs for verification

### Trust Indicators

When browsing packages on [npmjs.com](https://npmjs.com), look for:

- ✅ **Provenance badge**: Indicates the package has verified provenance
- 🔗 **Source link**: Direct link to the GitHub repository and commit
- 📊 **Build details**: Information about the CI/CD process used

## Implementation Details

### GitHub Actions Integration

Our publishing workflow automatically includes provenance when:

- Running on GitHub Actions (`GITHUB_ACTIONS=true`)
- Publishing to the public npm registry (not during dry runs)
- Proper OIDC permissions are configured

### Required Permissions

The workflow includes these permissions for provenance generation:

```yaml
permissions:
  id-token: write # Required for OIDC token generation
  contents: read # Required for repository access
```

### Publishing Command

The `--provenance` flag is automatically added to npm publish commands:

```bash
npm publish --provenance
```

## Troubleshooting

### Verification Failures

If `npm audit signatures` fails:

1. **Check npm version**: Requires npm 9.5.0 or later
   ```bash
   npm --version
   npm install -g npm@latest
   ```

2. **Network issues**: Ensure access to Sigstore's certificate transparency logs
   ```bash
   npm audit signatures --verbose
   ```

3. **Registry configuration**: Verify npm is configured for the correct registry
   ```bash
   npm config get registry
   ```

### Missing Provenance

If a package lacks provenance attestations:

- It may have been published before provenance was enabled
- It might have been published outside of GitHub Actions
- Check the package version - newer versions should include provenance

## Best Practices

### For Package Consumers

1. **Regular Verification**: Run `npm audit signatures` as part of your security workflow
2. **Check Provenance**: Verify packages come from expected sources
3. **Monitor Updates**: Be aware when packages lack provenance attestations
4. **Automate Checks**: Include signature verification in CI/CD pipelines

### For Package Publishers

1. **Enable Provenance**: Use `--provenance` flag when publishing
2. **Use Trusted CI/CD**: Publish from secure, auditable environments
3. **Keep Dependencies Updated**: Ensure npm and related tools are current
4. **Document Security**: Inform users about provenance availability

## Learn More

- [NPM Provenance Documentation](https://docs.npmjs.com/generating-provenance-statements)
- [SLSA Framework](https://slsa.dev/)
- [Sigstore Project](https://sigstore.dev/)
- [GitHub Blog: Introducing npm package provenance](https://github.blog/2023-04-19-introducing-npm-package-provenance/)

## Support

If you encounter issues with package verification or have questions about our provenance implementation:

1. Check this documentation for common solutions
2. Review the [GitHub Issues](https://github.com/tinovyatkin/serpen/issues) for known problems
3. Open a new issue with details about verification failures
4. Include npm version, package version, and error messages

---

_This documentation is maintained as part of the Serpen project's commitment to supply chain security and transparency._
