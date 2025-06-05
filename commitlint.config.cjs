module.exports = { 
  extends: ['@commitlint/config-conventional'],
  rules: {
    'scope-enum': [
      2,
      'always',
      [
        // Core components
        'parser',
        'bundler', 
        'resolver',
        'ast',
        'emit',
        'deps',
        'config',
        'cli',
        // Testing & CI
        'test',
        'ci',
        // Documentation & AI
        'docs',
        'ai',
        // Build & packaging
        'build',
        'npm',
        'pypi',
        'release'
      ]
    ]
  }
};