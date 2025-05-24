/* eslint-disable @typescript-eslint/no-var-requires */
const { build } = require('esbuild');
const path = require('path');

build({
  entryPoints: ['es5/server.js'],
  bundle: true,
  platform: 'node',
  outfile: 'dist/server.bundle.js',
  external: [
    // Exclude all DB drivers except 'pg' and 'pg-query-stream'
    'better-sqlite3',
    'mysql2',
    'mysql',
    'tedious',
    'sqlite3',
    'oracledb',
    // Node built-ins
    'fs',
    'path',
    'os',
    'http',
    'https',
    'stream',
    'crypto',
    'zlib',
    'util',
    'events',
    'child_process',
    'url',
    'net',
    'tls',
    'readline',
    'dns',
    'cluster',
    'module',
    'assert',
    'buffer',
    'console',
    'constants',
    'domain',
    'process',
    // 'punycode',
    'querystring',
    'repl',
    'string_decoder',
    'timers',
    'tty',
    'v8',
    'vm',
    'worker_threads',
    './xhr-sync-worker.js',
  ],
  sourcemap: true,
  target: 'node16',
  plugins: [
    {
      name: 'null-punycode',
      setup(build) {
        build.onResolve({ filter: /^punycode$/ }, (args) => ({
          path: path.resolve(__dirname, 'dist/null-module.js'),
        }));
      },
    },
  ],
}).catch(() => process.exit(1));
