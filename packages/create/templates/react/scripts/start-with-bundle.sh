#!/bin/sh

echo "Installing curl..."
apk add curl

echo "Starting Tonk server in background..."
tsx src/docker-start.ts &
SERVER_PID=$!

echo "Waiting for Tonk server to start..."
until curl --output /dev/null --silent --fail http://localhost:7777/ping; do
  printf "."
  sleep 1
done

echo "Tonk server is up!"

echo "Creating app bundle..."
cd /tmp && tar -czf app-bundle.tar.gz -C /tmp/app-bundle .

echo "Uploading app bundle to Tonk server..."
curl -X POST -F "bundle=@/tmp/app-bundle.tar.gz" -F "name=tonk-app" http://localhost:7777/upload-bundle

echo "Starting app bundle..."
curl -X POST -H "Content-Type: application/json" -d '{"bundleName":"tonk-app","port":8000}' http://localhost:7777/start

echo "Waiting for app bundle to be ready on port 8000..."
until curl --output /dev/null --silent --fail http://localhost:8000/; do
  printf "."
  sleep 1
done

echo "App started successfully and is responding on port 8000!"

# Keep the server running
wait $SERVER_PID
