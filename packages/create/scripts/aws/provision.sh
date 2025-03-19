#!/bin/bash


if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <key_pair_name>"
fi

# Get latest Amazon Linux 2 AMI ID
AMI_ID=$(aws ec2 describe-images \
    --owners amazon \
    --filters "Name=name,Values=amzn2-ami-hvm-*-x86_64-gp2" "Name=state,Values=available" \
    --query "Images | sort_by(@, &CreationDate)[-1].ImageId" \
    --output text)

echo "Latest AMI ID: $AMI_ID"

# Launch EC2 instance with user data script
INSTANCE_ID=$(aws ec2 run-instances \
    --image-id $AMI_ID \
    --instance-type t2.micro \
    --key-name $1 \
    --security-groups default \
    --query "Instances[0].InstanceId" \
    --output text)

echo "Instance launched with ID: $INSTANCE_ID"