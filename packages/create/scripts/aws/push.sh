#!/bin/bash

# Check if all required arguments are provided
if [ "$#" -ne 3 ]; then
    echo "Usage: $0 <pem_file> <source_folder> <ec2_instance>"
    echo "Example: $0 ~/my-key.pem ./my-project ec2-user@ec2-123.compute.amazonaws.com"
    exit 1
fi

PEM_FILE=$1
SOURCE_FOLDER=$2
EC2_INSTANCE=$3

# Check if PEM file exists
if [ ! -f "$PEM_FILE" ]; then
    echo "Error: PEM file not found at $PEM_FILE"
    exit 1
fi

# Check if source folder exists
if [ ! -d "$SOURCE_FOLDER" ]; then
    echo "Error: Source folder not found at $SOURCE_FOLDER"
    exit 1
fi

# Get the basename of the source folder
FOLDER_NAME=$(basename "$SOURCE_FOLDER")

echo "Copying files to EC2 instance..."
scp -i "$PEM_FILE" -r "$SOURCE_FOLDER" "$EC2_INSTANCE:~/"

if [ $? -eq 0 ]; then
    echo "Files copied successfully"
    echo "Executing start.sh on remote instance..."
    
    # SSH into the instance and execute start.sh
    ssh -i "$PEM_FILE" "$EC2_INSTANCE" "cd ~/$FOLDER_NAME && ./start.sh"
else
    echo "Error: Failed to copy files to EC2 instance"
    exit 1
fi
