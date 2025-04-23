const path = require('path');
const HtmlWebpackPlugin = require('html-webpack-plugin');
const CopyPlugin = require('copy-webpack-plugin');
const { InjectManifest } = require('workbox-webpack-plugin');
const webpack = require('webpack');

module.exports = (env, argv) => {
  const isProduction = argv.mode === 'production';
  
  // Basic config that's always applied
  const config = {
    entry: './src/index.tsx',
    devtool: 'inline-source-map',
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
        url: false
      }
    },
    output: {
      filename: 'bundle.js',
      path: path.resolve(__dirname, 'dist'),
      publicPath: '/'
    },
    plugins: [
      new HtmlWebpackPlugin({
        template: './public/index.html',
        inject: true,
        // In development mode, add the cleanup script
        ...(isProduction ? {} : {
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
          }
        ],
      }),
      // Define environment variables that will be available to the application
      new webpack.DefinePlugin({
        'process.env.NODE_ENV': JSON.stringify(isProduction ? 'production' : 'development')
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
        target: 'http://localhost:8080',
        ws: true,
        changeOrigin: true
      }],
    },
    experiments: { asyncWebAssembly: true },
    target: 'web',
    performance: {
      hints: false,
      maxEntrypointSize: 512000,
      maxAssetSize: 512000,
    },
  };

  // Only add Workbox in production mode to avoid development issues
  if (isProduction) {
    config.plugins.push(
      new InjectManifest({
        swSrc: './src/service-worker.ts',
        swDest: 'service-worker.js',
        exclude: [/\.map$/, /asset-manifest\.json$/],
        maximumFileSizeToCacheInBytes: 10 * 1024 * 1024, // 10 MB
      })
    );
  }

  return config;
};
