#!/usr/bin/env node

const { execFileSync } = require('child_process');
const { existsSync } = require('fs');
const path = require('path');

/**
 * Detect if we're running on musl libc (like Alpine Linux)
 * @returns {boolean} true if musl is detected
 */
function detectMusl() {
  try {
    // Check for musl-specific files/processes
    if (existsSync('/lib/ld-musl-x86_64.so.1') ||
      existsSync('/lib/ld-musl-aarch64.so.1') ||
      existsSync('/usr/lib/libc.musl-x86_64.so.1') ||
      existsSync('/usr/lib/libc.musl-aarch64.so.1')) {
      return true;
    }

    // Try to read /proc/version for musl indicators
    if (existsSync('/proc/version')) {
      const fs = require('fs');
      const version = fs.readFileSync('/proc/version', 'utf8');
      if (version.includes('musl')) {
        return true;
      }
    }

    return false;
  } catch (error) {
    // If we can't determine, default to glibc
    return false;
  }
}

/**
 * Get the platform-specific package name for the current system
 * @returns {string} the npm package name for this platform
 */
function getPlatformPackageName() {
  const platform = process.platform;
  const arch = process.arch;

  // Map Node.js platform/arch to our package naming convention
  let pkgPlatform;
  let pkgArch;
  let suffix = '';

  switch (platform) {
    case 'linux':
      pkgPlatform = 'linux';
      suffix = detectMusl() ? '-musl' : '-gnu';
      break;
    case 'darwin':
      pkgPlatform = 'darwin';
      break;
    case 'win32':
      pkgPlatform = 'win32';
      break;
    default:
      throw new Error(`Unsupported platform: ${platform}`);
  }

  switch (arch) {
    case 'x64':
      pkgArch = 'x64';
      break;
    case 'arm64':
      pkgArch = 'arm64';
      break;
    case 'ia32':
      if (platform === 'win32') {
        pkgArch = 'ia32';
      } else {
        throw new Error(`Unsupported architecture ${arch} for platform ${platform}`);
      }
      break;
    default:
      throw new Error(`Unsupported architecture: ${arch}`);
  }

  return `@cribo/${pkgPlatform}-${pkgArch}${suffix}`;
}

/**
 * Find and execute the platform-specific Serpen binary
 */
function main() {
  try {
    const pkgName = getPlatformPackageName();
    const binName = process.platform === 'win32' ? 'cribo.exe' : 'cribo';

    // Try to resolve the binary path from the platform package
    let binPath;
    try {
      binPath = require.resolve(`${pkgName}/bin/${binName}`);
    } catch (resolveError) {
      // Platform package not found or binary missing
      console.error(`Error: Could not find Cribo binary for your platform (${pkgName}).`);
      console.error('');
      console.error('This usually means:');
      console.error('1. Optional dependencies were disabled during installation');
      console.error('2. Your platform is not supported');
      console.error('');
      console.error('To fix this:');
      console.error('1. Reinstall with optional dependencies enabled:');
      console.error('   npm install cribo');
      console.error('   # or');
      console.error('   yarn add cribo');
      console.error('');
      console.error('2. If you disabled optional dependencies, re-enable them:');
      console.error('   npm install --include=optional');
      console.error('');
      console.error(`Expected package: ${pkgName}`);
      console.error(`Platform: ${process.platform} ${process.arch}`);
      process.exit(1);
    }

    // Verify the binary exists and is executable
    if (!existsSync(binPath)) {
      console.error(`Error: Binary not found at ${binPath}`);
      console.error('The platform package was installed but the binary is missing.');
      console.error('Please try reinstalling cribo.');
      process.exit(1);
    }

    // Execute the binary with the same arguments passed to this script
    // Skip the first two arguments (node and script path)
    const args = process.argv.slice(2);

    try {
      execFileSync(binPath, args, {
        stdio: 'inherit',  // Forward stdin/stdout/stderr to the user
        windowsHide: false // On Windows, don't hide the console window
      });
    } catch (execError) {
      // If the binary exits with a non-zero code, preserve that exit code
      if (execError.status !== undefined) {
        process.exit(execError.status);
      }
      // If there was an execution error (e.g., binary corrupted), report it
      console.error(`Error executing Cribo binary: ${execError.message}`);
      process.exit(1);
    }

  } catch (error) {
    console.error(`Error: ${error.message}`);
    process.exit(1);
  }
}

// Only run if this script is executed directly (not required as a module)
if (require.main === module) {
  main();
}

module.exports = { getPlatformPackageName, detectMusl };
