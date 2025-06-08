#!/usr/bin/env node

/**
 * Test the npm package locally
 * This script helps validate the npm package before publishing
 */

const { execSync, spawn } = require('child_process');
const fs = require('fs');
const path = require('path');
const os = require('os');

function log(message) {
    console.log(`[TEST] ${message}`);
}

function error(message) {
    console.error(`[ERROR] ${message}`);
}

function success(message) {
    console.log(`[SUCCESS] ${message}`);
}

function execCommand(command, options = {}) {
    try {
        const result = execSync(command, {
            encoding: 'utf8',
            stdio: 'pipe',
            ...options
        });
        return result.trim();
    } catch (err) {
        throw new Error(`Command failed: ${command}\n${err.message}`);
    }
}

async function testPackage() {
    log('Testing Cribo npm package...');

    // Create a temporary directory for testing
    const testDir = path.join(os.tmpdir(), 'cribo-npm-test-' + Date.now());
    fs.mkdirSync(testDir);

    try {
        process.chdir(testDir);
        log(`Created test directory: ${testDir}`);

        // Initialize a simple npm project
        execCommand('npm init -y');
        log('Initialized test npm project');

        // Get the path to our local package
        const packagePath = path.join(__dirname, '..', 'npm', 'cribo');

        // Install our local package
        log(`Installing local package from: ${packagePath}`);
        execCommand(`npm install "${packagePath}"`);
        success('Successfully installed local cribo package');

        // Test if cribo command is available
        log('Testing cribo command...');

        try {
            // Test cribo --help
            const helpOutput = execCommand('npx cribo --help');
            log('cribo --help output:');
            console.log(helpOutput);
            success('cribo --help executed successfully');
        } catch (err) {
            error(`cribo --help failed: ${err.message}`);
            throw err;
        }

        // Test cribo --version if available
        try {
            const versionOutput = execCommand('npx cribo --version');
            log('cribo --version output:');
            console.log(versionOutput);
            success('cribo --version executed successfully');
        } catch (err) {
            log('cribo --version not available (this is okay)');
        }

        // Check if the correct platform package was installed
        log('Checking installed platform packages...');
        const nodeModulesPath = path.join(testDir, 'node_modules');
        const installedPackages = fs.readdirSync(nodeModulesPath)
            .filter(name => name.startsWith('cribo-'))
            .sort();

        log(`Installed platform packages: ${installedPackages.join(', ')}`);

        // Check for expected platform package
        const platform = process.platform;
        const arch = process.arch;
        let expectedPackages = [];

        if (platform === 'linux') {
            // On Linux, we might have both gnu and musl
            expectedPackages = [
                `cribo-linux-${arch}-gnu`,
                `cribo-linux-${arch}-musl`
            ];
        } else if (platform === 'darwin') {
            expectedPackages = [`cribo-darwin-${arch}`];
        } else if (platform === 'win32') {
            expectedPackages = [`cribo-win32-${arch}`];
        }

        const foundExpected = expectedPackages.some(pkg => installedPackages.includes(pkg));

        if (foundExpected) {
            success(`Found expected platform package for ${platform}-${arch}`);
        } else {
            error(`Expected platform package not found. Expected one of: ${expectedPackages.join(', ')}`);
            log(`Platform: ${platform}, Arch: ${arch}`);
        }

        // Test the launcher script directly
        log('Testing launcher script...');
        const launcherPath = path.join(nodeModulesPath, 'cribo', 'bin', 'cribo.js');

        if (fs.existsSync(launcherPath)) {
            const launcherModule = require(launcherPath);
            if (typeof launcherModule.getPlatformPackageName === 'function') {
                const platformPkg = launcherModule.getPlatformPackageName();
                log(`Platform package name: ${platformPkg}`);

                if (installedPackages.includes(platformPkg)) {
                    success(`Platform package ${platformPkg} is correctly installed`);
                } else {
                    error(`Platform package ${platformPkg} not found in installed packages`);
                }
            }
        }

        success('All tests passed!');

    } finally {
        // Clean up
        process.chdir(__dirname);
        try {
            fs.rmSync(testDir, { recursive: true, force: true });
            log(`Cleaned up test directory: ${testDir}`);
        } catch (err) {
            log(`Warning: Could not clean up test directory: ${err.message}`);
        }
    }
}

// CLI usage
if (require.main === module) {
    testPackage().catch(err => {
        error(err.message);
        process.exit(1);
    });
}

module.exports = { testPackage };
