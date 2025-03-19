#!/bin/bash

# This script creates an AWS EC2 key pair and saves it to a file

if [ $# -lt 1 ]; then
  echo "Usage: $0 <key-name> [aws-profile-option] [aws-region-option]"
  exit 1
fi

KEY_NAME=$1
PROFILE_OPTION=$2
REGION_OPTION=$3

# Create the key pair and save it to a file
aws ec2 create-key-pair ${PROFILE_OPTION} ${REGION_OPTION} --key-name ${KEY_NAME} --query 'KeyMaterial' --output text > ${KEY_NAME}.pem

# Set appropriate permissions for the key file
chmod 400 ${KEY_NAME}.pem

echo "Key pair ${KEY_NAME} created and saved."
