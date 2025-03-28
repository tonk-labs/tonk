const path = require('path');
const HtmlWebpackPlugin = require('html-webpack-plugin');
const CopyPlugin = require('copy-webpack-plugin');
const { InjectManifest } = require('workbox-webpack-plugin');
const webpack = require('webpack');

module.exports = (env, argv) => {
  const isProduction = argv.mode === 'production';
  const isElectron = env && env.ELECTRON === 'true';
  
  // Basic config that's always applied
  const config = {
    entry: './src/index.tsx',
    devtool: isProduction ? 'source-map' : 'inline-source-map',
    module: {
      rules: [
        {
          test: /\.(ts|tsx)$/,
          exclude: /node_modules/,
          use: {
            loader: 'ts-loader',
            options: {
              transpileOnly: true
            }
          }
        },
        {
          test: /\.css$/,
          use: ['style-loader', 'css-loader', 'postcss-loader']
        },
        {
          test: /\.wasm$/,
          type: "asset/resource",
          generator: {
            filename: '[name][ext]'
          }
        }
      ]
    },
    resolve: {
      extensions: ['.tsx', '.ts', '.js', '.jsx', '.mjs'],
      extensionAlias: {
        '.js': ['.js', '.ts', '.tsx', '.jsx'],
        '.mjs': ['.mjs', '.js', '.ts', '.tsx', '.jsx']
      },
      fallback: {
        path: false,
        fs: false,
        os: false,
        crypto: false,
        url: false,
        wbg: false
      },
      alias: {
        '@automerge/automerge': path.resolve(__dirname, 'node_modules/@automerge/automerge/dist/mjs')
      }
    },
    output: {
      filename: 'bundle.js',
      path: path.resolve(__dirname, 'dist'),
      publicPath: isElectron ? './' : '/', // Use relative paths for Electron
    },
    plugins: [
      new HtmlWebpackPlugin({
        template: './public/index.html',
        inject: true,
        // In development mode, add the cleanup script if not in Electron
        ...(isProduction || isElectron ? {} : {
          scripts: [
            { src: '/sw-cleanup.js', async: true, defer: true }
          ]
        })
      }),
      new CopyPlugin({
        patterns: [
          { 
            from: 'public', 
            to: '', 
            globOptions: {
              ignore: ['**/index.html']
            }
          },
          {
            from: 'node_modules/@automerge/automerge/dist/mjs/wasm_bindgen_output/web/automerge_wasm_bg.wasm',
            to: 'automerge_wasm_bg.wasm',
            transform(content) {
              return content;
            },
          }
        ],
      }),
      // Define environment variables that will be available to the application
      new webpack.DefinePlugin({
        'process.env.NODE_ENV': JSON.stringify(isProduction ? 'production' : 'development'),
        'process.env.IS_ELECTRON': JSON.stringify(isElectron ? 'true' : 'false')
      }),
    ],
    devServer: {
      static: {
        directory: path.join(__dirname, 'public'),
      },
      compress: true,
      historyApiFallback: true,
      hot: true,
      port: 3000,
      proxy: [{
        context: ['/sync', '/api'],
        target: 'http://localhost:4080',
        ws: true,
        changeOrigin: true
      }],
    },
    experiments: { asyncWebAssembly: true },
    target: isElectron ? 'electron-renderer' : 'web',
    performance: {
      hints: false,
      maxEntrypointSize: 512000,
      maxAssetSize: 512000,
    },
  };

  // Only add Workbox in production mode and not for Electron
  if (isProduction && !isElectron) {
    config.plugins.push(
      new InjectManifest({
        swSrc: './src/service-worker.ts',
        swDest: 'service-worker.js',
        exclude: [/\.map$/, /asset-manifest\.json$/],
        maximumFileSizeToCacheInBytes: 10 * 1024 * 1024, // 10 MB
      })
    );
  }

  // For Electron builds, add nodeIntegration settings
  if (isElectron) {
    // Make sure 'util' and other Node.js built-ins are properly handled
    config.externals = {
      ...config.externals,
      util: 'commonjs util',
      path: 'commonjs path',
      fs: 'commonjs fs',
      os: 'commonjs os'
    };
    
    // Ensure proper target
    config.target = 'electron-renderer';
    
    // Enhanced WebAssembly configuration for Electron
    config.experiments = {
      ...config.experiments,
      asyncWebAssembly: true,
      syncWebAssembly: true,
      topLevelAwait: true
    };

    // Add additional WASM-specific configuration for Electron
    config.resolve.fallback = {
      ...config.resolve.fallback,
      wbg: false,
      buffer: require.resolve('buffer/')
    };
  }

  return config;
};
