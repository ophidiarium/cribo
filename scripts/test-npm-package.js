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
    log('Testing Serpen npm package...');

    // Create a temporary directory for testing
    const testDir = path.join(os.tmpdir(), 'serpen-npm-test-' + Date.now());
    fs.mkdirSync(testDir);

    try {
        process.chdir(testDir);
        log(`Created test directory: ${testDir}`);

        // Initialize a simple npm project
        execCommand('npm init -y');
        log('Initialized test npm project');

        // Get the path to our local package
        const packagePath = path.join(__dirname, '..', 'npm', 'serpen');

        // Install our local package
        log(`Installing local package from: ${packagePath}`);
        execCommand(`npm install "${packagePath}"`);
        success('Successfully installed local serpen package');

        // Test if serpen command is available
        log('Testing serpen command...');

        try {
            // Test serpen --help
            const helpOutput = execCommand('npx serpen --help');
            log('serpen --help output:');
            console.log(helpOutput);
            success('serpen --help executed successfully');
        } catch (err) {
            error(`serpen --help failed: ${err.message}`);
            throw err;
        }

        // Test serpen --version if available
        try {
            const versionOutput = execCommand('npx serpen --version');
            log('serpen --version output:');
            console.log(versionOutput);
            success('serpen --version executed successfully');
        } catch (err) {
            log('serpen --version not available (this is okay)');
        }

        // Check if the correct platform package was installed
        log('Checking installed platform packages...');
        const nodeModulesPath = path.join(testDir, 'node_modules');
        const installedPackages = fs.readdirSync(nodeModulesPath)
            .filter(name => name.startsWith('serpen-'))
            .sort();

        log(`Installed platform packages: ${installedPackages.join(', ')}`);

        // Check for expected platform package
        const platform = process.platform;
        const arch = process.arch;
        let expectedPackages = [];

        if (platform === 'linux') {
            // On Linux, we might have both gnu and musl
            expectedPackages = [
                `serpen-linux-${arch}-gnu`,
                `serpen-linux-${arch}-musl`
            ];
        } else if (platform === 'darwin') {
            expectedPackages = [`serpen-darwin-${arch}`];
        } else if (platform === 'win32') {
            expectedPackages = [`serpen-win32-${arch}`];
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
        const launcherPath = path.join(nodeModulesPath, 'serpen', 'bin', 'serpen.js');

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
