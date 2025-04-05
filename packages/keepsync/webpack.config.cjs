const path = require('path');

module.exports = {
  entry: './src/index.ts',
  mode: 'production',
  devtool: 'source-map',
  module: {
    rules: [
      {
        test: /\.tsx?$/,
        use: {
          loader: 'ts-loader',
          options: {
            compilerOptions: {
              moduleResolution: 'node',
            },
          },
        },
        exclude: /node_modules/,
      },
    ],
  },
  resolve: {
    extensions: ['.tsx', '.ts', '.js'],
    extensionAlias: {
      '.js': ['.js', '.ts'],
      '.cjs': ['.cjs', '.cts'],
      '.mjs': ['.mjs', '.mts'],
    },
    fullySpecified: false,
    alias: {
      'node-fetch': false,
      'ws': false,
      'fake-indexeddb': false,
      'buffer': false
    },
    fallback: {
      'buffer': false
    }
  },
  output: {
    filename: 'index.js',
    path: path.resolve(__dirname, 'dist'),
    clean: false, // Changed to false to preserve Node build
    library: {
      type: 'module',
    },
    module: true,
    environment: {
      module: true,
    },
  },
  experiments: {
    outputModule: true,
    asyncWebAssembly: true,
  },
  externals: {
    'react': 'react',
    'zustand': 'zustand',
    '@automerge/automerge': '@automerge/automerge',
    'node-fetch': 'node-fetch',
    'ws': 'ws',
    'fake-indexeddb': 'fake-indexeddb'
  },
  optimization: {
    minimize: false,
  },
  target: ['web', 'es2020'],
  plugins: [],
};
