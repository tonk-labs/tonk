aws ec2 describe-instances \
    --instance-ids "$1" \
    --query "Reservations[*].Instances[*].PublicDnsName" \
    --output text