#!/usr/bin/env node

/**
 * Generate platform-specific npm packages from template
 * Usage: node scripts/generate-npm-packages.js <version> <target-dir> <binaries-dir>
 */

const fs = require('fs');
const path = require('path');

// Platform mappings from Rust targets to npm os/cpu values
const PLATFORM_MAPPINGS = [
  {
    rustTarget: 'x86_64-unknown-linux-gnu',
    nodePkg: '@cribo/linux-x64-gnu',
    nodeOs: 'linux',
    nodeArch: 'x64',
    extension: '',
    binaryName: 'cribo'
  },
  {
    rustTarget: 'x86_64-unknown-linux-musl',
    nodePkg: '@cribo/linux-x64-musl',
    nodeOs: 'linux',
    nodeArch: 'x64',
    extension: '',
    binaryName: 'cribo'
  },
  {
    rustTarget: 'aarch64-unknown-linux-gnu',
    nodePkg: '@cribo/linux-arm64-gnu',
    nodeOs: 'linux',
    nodeArch: 'arm64',
    extension: '',
    binaryName: 'cribo'
  },
  {
    rustTarget: 'aarch64-unknown-linux-musl',
    nodePkg: '@cribo/linux-arm64-musl',
    nodeOs: 'linux',
    nodeArch: 'arm64',
    extension: '',
    binaryName: 'cribo'
  },
  {
    rustTarget: 'x86_64-apple-darwin',
    nodePkg: '@cribo/darwin-x64',
    nodeOs: 'darwin',
    nodeArch: 'x64',
    extension: '',
    binaryName: 'cribo'
  },
  {
    rustTarget: 'aarch64-apple-darwin',
    nodePkg: '@cribo/darwin-arm64',
    nodeOs: 'darwin',
    nodeArch: 'arm64',
    extension: '',
    binaryName: 'cribo'
  },
  {
    rustTarget: 'x86_64-pc-windows-msvc',
    nodePkg: '@cribo/win32-x64',
    nodeOs: 'win32',
    nodeArch: 'x64',
    extension: '.exe',
    binaryName: 'cribo.exe'
  },
  {
    rustTarget: 'aarch64-pc-windows-msvc',
    nodePkg: '@cribo/win32-arm64',
    nodeOs: 'win32',
    nodeArch: 'arm64',
    extension: '.exe',
    binaryName: 'cribo.exe'
  }
];

function generatePackages(version, targetDir, binariesDir) {
  console.log(`Generating npm packages version ${version}`);
  console.log(`Target directory: ${targetDir}`);
  console.log(`Binaries directory: ${binariesDir}`);

  // Read the template
  const templatePath = path.join(__dirname, '..', 'npm', 'package.json.tmpl');
  const template = fs.readFileSync(templatePath, 'utf8');

  // Ensure target directory exists
  if (!fs.existsSync(targetDir)) {
    fs.mkdirSync(targetDir, { recursive: true });
  }

  const generatedPackages = [];

  for (const platform of PLATFORM_MAPPINGS) {
    const { rustTarget, nodePkg, nodeOs, nodeArch, extension, binaryName } = platform;

    // Check if binary exists for this platform - look in target-specific directory first
    let binaryPath = path.join(binariesDir, rustTarget, binaryName);
    if (!fs.existsSync(binaryPath)) {
      // Fallback to flat structure for backward compatibility
      binaryPath = path.join(binariesDir, binaryName);
      if (!fs.existsSync(binaryPath)) {
        console.warn(`Warning: Binary not found for ${rustTarget} at either:
  - ${path.join(binariesDir, rustTarget, binaryName)}
  - ${path.join(binariesDir, binaryName)}`);
        continue;
      }
    }

    // Create package directory - handle scoped packages properly
    let pkgDir;
    if (nodePkg.startsWith('@')) {
      // For scoped packages like @cribo/darwin-arm64, create @cribo/darwin-arm64/
      const [scope, packageName] = nodePkg.split('/');
      const scopeDir = path.join(targetDir, scope);
      pkgDir = path.join(scopeDir, packageName);
    } else {
      // For regular packages like cribo-darwin-arm64
      pkgDir = path.join(targetDir, nodePkg);
    }

    const binDir = path.join(pkgDir, 'bin');

    fs.mkdirSync(pkgDir, { recursive: true });
    fs.mkdirSync(binDir, { recursive: true });

    // Generate package.json from template
    const packageJson = template
      .replace(/\$\{node_pkg\}/g, nodePkg)
      .replace(/\$\{node_version\}/g, version)
      .replace(/\$\{node_os\}/g, nodeOs)
      .replace(/\$\{node_arch\}/g, nodeArch)
      .replace(/\$\{extension\}/g, extension);

    // Write package.json
    fs.writeFileSync(path.join(pkgDir, 'package.json'), packageJson);

    // Copy binary
    const targetBinaryPath = path.join(binDir, binaryName);
    fs.copyFileSync(binaryPath, targetBinaryPath);

    // Make binary executable on Unix-like systems
    if (process.platform !== 'win32') {
      fs.chmodSync(targetBinaryPath, 0o755);
    }

    // Create a simple README
    const readmeContent = `# ${nodePkg}

This package contains the Cribo binary for ${nodeOs}-${nodeArch}.

This package is automatically installed as an optional dependency when you install the main \`cribo\` package.

For more information, visit: https://github.com/ophidiarium/cribo
`;

    fs.writeFileSync(path.join(pkgDir, 'README.md'), readmeContent);

    generatedPackages.push({
      name: nodePkg,
      path: pkgDir,
      platform: rustTarget
    });

    console.log(`✓ Generated package: ${nodePkg}`);
  }

  console.log(`\nGenerated ${generatedPackages.length} packages:`);
  generatedPackages.forEach(pkg => {
    console.log(`  ${pkg.name} (${pkg.platform})`);
  });

  return generatedPackages;
}

// CLI usage
if (require.main === module) {
  const args = process.argv.slice(2);

  if (args.length !== 3) {
    console.error('Usage: node generate-npm-packages.js <version> <target-dir> <binaries-dir>');
    console.error('');
    console.error('Example:');
    console.error('  node scripts/generate-npm-packages.js 0.3.0 ./npm-dist ./target/release');
    process.exit(1);
  }

  const [version, targetDir, binariesDir] = args;

  try {
    generatePackages(version, targetDir, binariesDir);
    console.log('\nAll packages generated successfully!');
  } catch (error) {
    console.error(`Error: ${error.message}`);
    process.exit(1);
  }
}

module.exports = { generatePackages, PLATFORM_MAPPINGS };
