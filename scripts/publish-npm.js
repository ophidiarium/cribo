#!/usr/bin/env node

/**
 * NPM publishing script for Serpen
 * Publishes both the base package and all platform-specific packages
 */

const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

function execCommand(command, options = {}) {
  console.log(`Running: ${command}`);
  try {
    return execSync(command, {
      stdio: 'inherit',
      encoding: 'utf8',
      ...options
    });
  } catch (error) {
    console.error(`Command failed: ${command}`);
    throw error;
  }
}

function updatePackageVersion(packagePath, version) {
  const packageJsonPath = path.join(packagePath, 'package.json');
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));

  // Update version
  packageJson.version = version;

  // Update optionalDependencies versions if this is the base package
  if (packageJson.optionalDependencies) {
    for (const dep in packageJson.optionalDependencies) {
      packageJson.optionalDependencies[dep] = version;
    }
  }

  fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2) + '\n');
  console.log(`Updated ${packageJsonPath} to version ${version}`);
}

function publishPackage(packagePath, tag = 'latest', dryRun = false) {
  const packageJson = JSON.parse(fs.readFileSync(path.join(packagePath, 'package.json'), 'utf8'));
  const packageName = packageJson.name;

  console.log(`\n📦 Publishing ${packageName}...`);

  const publishCmd = [
    'npm publish',
    packagePath,
    `--tag ${tag}`,
    '--access public'
  ];

  if (dryRun) {
    publishCmd.push('--dry-run');
  }

  try {
    execCommand(publishCmd.join(' '));
    console.log(`✅ Successfully published ${packageName}`);
    return true;
  } catch (error) {
    if (error.message.includes('cannot publish over the previously published versions')) {
      console.log(`⚠️  Version ${packageJson.version} of ${packageName} already exists, skipping...`);
      return true;
    }
    console.error(`❌ Failed to publish ${packageName}: ${error.message}`);
    throw error;
  }
}

function main() {
  const args = process.argv.slice(2);
  const version = args[0];
  const npmDistDir = args[1] || './npm-dist';
  const dryRun = args.includes('--dry-run');
  const tag = args.includes('--tag') ? args[args.indexOf('--tag') + 1] : 'latest';

  if (!version) {
    console.error('Usage: node publish-npm.js <version> [npm-dist-dir] [--dry-run] [--tag <tag>]');
    console.error('');
    console.error('Example:');
    console.error('  node scripts/publish-npm.js 0.3.0 ./npm-dist');
    console.error('  node scripts/publish-npm.js 0.3.0 ./npm-dist --dry-run');
    console.error('  node scripts/publish-npm.js 0.3.0 ./npm-dist --tag beta');
    process.exit(1);
  }

  console.log(`🚀 Publishing Serpen npm packages version ${version}`);
  console.log(`📁 Distribution directory: ${npmDistDir}`);
  console.log(`🏷️  Tag: ${tag}`);
  console.log(`🧪 Dry run: ${dryRun ? 'Yes' : 'No'}`);
  console.log('');

  // Update base package version
  const basePackagePath = path.join(__dirname, '..', 'npm', 'serpen');
  updatePackageVersion(basePackagePath, version);

  // Collect all platform packages
  const platformPackages = [];
  if (fs.existsSync(npmDistDir)) {
    const entries = fs.readdirSync(npmDistDir);
    for (const entry of entries) {
      const entryPath = path.join(npmDistDir, entry);

      if (fs.statSync(entryPath).isDirectory()) {
        if (entry.startsWith('@')) {
          // Handle scoped packages - look inside the scope directory
          const scopeEntries = fs.readdirSync(entryPath);
          for (const scopeEntry of scopeEntries) {
            const packagePath = path.join(entryPath, scopeEntry);
            const packageJsonPath = path.join(packagePath, 'package.json');

            if (fs.statSync(packagePath).isDirectory() && fs.existsSync(packageJsonPath)) {
              const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
              if (packageJson.name.startsWith('@serpen/') || packageJson.name.startsWith('serpen-')) {
                platformPackages.push({
                  name: packageJson.name,
                  path: packagePath
                });
              }
            }
          }
        } else {
          // Handle regular packages
          const packagePath = entryPath;
          const packageJsonPath = path.join(packagePath, 'package.json');

          if (fs.existsSync(packageJsonPath)) {
            const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
            if (packageJson.name.startsWith('@serpen/') || packageJson.name.startsWith('serpen-')) {
              platformPackages.push({
                name: packageJson.name,
                path: packagePath
              });
            }
          }
        }
      }
    }
  }

  console.log(`Found ${platformPackages.length} platform packages:`);
  platformPackages.forEach(pkg => console.log(`  - ${pkg.name}`));
  console.log('');

  // Publish platform packages first
  let successCount = 0;
  let failureCount = 0;

  for (const pkg of platformPackages) {
    try {
      publishPackage(pkg.path, tag, dryRun);
      successCount++;
    } catch (error) {
      console.error(`Failed to publish ${pkg.name}`);
      failureCount++;
    }
  }

  // Only publish base package if all platform packages succeeded (or dry run)
  if (failureCount === 0 || dryRun) {
    try {
      publishPackage(basePackagePath, tag, dryRun);
      successCount++;
      console.log('\n🎉 All packages published successfully!');
    } catch (error) {
      console.error('Failed to publish base package');
      failureCount++;
    }
  } else {
    console.error('\n❌ Skipping base package due to platform package failures');
  }

  console.log(`\n📊 Summary:`);
  console.log(`  ✅ Successful: ${successCount}`);
  console.log(`  ❌ Failed: ${failureCount}`);
  console.log(`  📦 Total: ${successCount + failureCount}`);

  if (failureCount > 0) {
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = { publishPackage, updatePackageVersion };
