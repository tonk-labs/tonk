#!/bin/bash

# This script provisions an EC2 instance using a key pair

if [ $# -lt 1 ]; then
  echo "Usage: $0 <key-name> [aws-profile-option] [aws-region-option]"
  exit 1
fi

KEY_NAME=$1
PROFILE_OPTION=$2
REGION_OPTION=$3

# Extract region from the region option (simpler method)
REGION=$(echo ${REGION_OPTION} | sed -n 's/.*--region \([a-z0-9-]*\).*/\1/p')

# If region is not found in the option, use default
if [ -z "$REGION" ]; then
  # Try to get the default region
  REGION=$(aws configure get region ${PROFILE_OPTION})
  
  # If still no region, use us-east-1 as fallback
  if [ -z "$REGION" ]; then
    REGION="us-east-1"
  fi
fi

echo "Using AWS region: ${REGION}"

# Query for the latest Amazon Linux 2023 AMI for the current region
echo "Querying for latest Amazon Linux 2023 AMI in region ${REGION}..."
AMI_ID=$(aws ec2 describe-images ${PROFILE_OPTION} ${REGION_OPTION} \
  --owners amazon \
  --filters "Name=name,Values=al2023-ami-2023.*-x86_64" "Name=state,Values=available" \
  --query "sort_by(Images, &CreationDate)[-1].ImageId" \
  --output text)

# If we couldn't get an AMI ID, exit with error
if [ -z "$AMI_ID" ] || [ "$AMI_ID" == "None" ]; then
  echo "ERROR: Unable to find a valid Amazon Linux 2023 AMI for region ${REGION}"
  exit 1
fi

echo "Using AMI ID: ${AMI_ID}"

# For eu-north-1 region, use t3.micro as t2.micro is not supported
INSTANCE_TYPE="t2.micro"
if [ "$REGION" = "eu-north-1" ]; then
  INSTANCE_TYPE="t3.micro"
fi

echo "Using instance type: ${INSTANCE_TYPE}"

# Create a custom security group
SECURITY_GROUP_NAME="tonk-security-group-$(date +%s)"
echo "Creating security group: ${SECURITY_GROUP_NAME}"

SECURITY_GROUP_ID=$(aws ec2 create-security-group ${PROFILE_OPTION} ${REGION_OPTION} \
  --group-name ${SECURITY_GROUP_NAME} \
  --description "Security group for Tonk" \
  --query 'GroupId' \
  --output text)

echo "Security group created with ID: ${SECURITY_GROUP_ID}"

# Add SSH access rule to the security group
aws ec2 authorize-security-group-ingress ${PROFILE_OPTION} ${REGION_OPTION} \
  --group-id ${SECURITY_GROUP_ID} \
  --protocol tcp \
  --port 22 \
  --cidr 0.0.0.0/0

echo "Added SSH access rule to security group"

# Add HTTPS access rule to the security group
aws ec2 authorize-security-group-ingress ${PROFILE_OPTION} ${REGION_OPTION} \
  --group-id ${SECURITY_GROUP_ID} \
  --protocol tcp \
  --port 443 \
  --cidr 0.0.0.0/0

echo "Added HTTPS access rule to security group"

# Launch an EC2 instance with Amazon Linux 2023 AMI and the new security group
INSTANCE_ID=$(aws ec2 run-instances ${PROFILE_OPTION} ${REGION_OPTION} \
  --image-id ${AMI_ID} \
  --instance-type ${INSTANCE_TYPE} \
  --key-name ${KEY_NAME} \
  --security-group-ids ${SECURITY_GROUP_ID} \
  --query 'Instances[0].InstanceId' \
  --output text)

echo "Instance launched with ID: ${INSTANCE_ID}"
