#!/bin/bash

# This script retrieves the public DNS name of an EC2 instance

if [ $# -lt 1 ]; then
  echo "Usage: $0 <instance-id> [aws-profile-option] [aws-region-option]"
  exit 1
fi

INSTANCE_ID=$1
PROFILE_OPTION=$2
REGION_OPTION=$3

# Get the public DNS name of the instance
PUBLIC_DNS=$(aws ec2 describe-instances ${PROFILE_OPTION} ${REGION_OPTION} \
  --instance-ids ${INSTANCE_ID} \
  --query 'Reservations[0].Instances[0].PublicDnsName' \
  --output text)

echo ${PUBLIC_DNS}
