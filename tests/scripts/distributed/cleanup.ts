#!/usr/bin/env tsx

import {
  EC2Client,
  DescribeInstancesCommand,
  TerminateInstancesCommand,
  DescribeSecurityGroupsCommand,
  DeleteSecurityGroupCommand,
} from '@aws-sdk/client-ec2';

const REGION = process.env.AWS_REGION || 'eu-north-1';

async function cleanup() {
  console.log('🧹 Cleaning up distributed test resources...');

  const ec2 = new EC2Client({ region: REGION });

  console.log('\n1. Finding test instances...');
  const describeCommand = new DescribeInstancesCommand({
    Filters: [
      {
        Name: 'tag:Project',
        Values: ['tonk-load-test'],
      },
      {
        Name: 'instance-state-name',
        Values: ['running', 'pending', 'stopping', 'stopped'],
      },
    ],
  });

  const instances = await ec2.send(describeCommand);
  const instanceIds =
    instances.Reservations?.flatMap(
      r => r.Instances?.map(i => i.InstanceId).filter(Boolean) || []
    ) || [];

  if (instanceIds.length > 0) {
    console.log(`Found ${instanceIds.length} test instances:`);
    instanceIds.forEach(id => console.log(`  - ${id}`));

    console.log('\n2. Terminating instances...');
    const terminateCommand = new TerminateInstancesCommand({
      InstanceIds: instanceIds as string[],
    });
    await ec2.send(terminateCommand);
    console.log('✓ Termination initiated');

    console.log('\n3. Waiting for instances to terminate...');
    await new Promise(resolve => setTimeout(resolve, 30000));
  } else {
    console.log('No test instances found');
  }

  console.log('\n4. Finding test security groups...');
  const sgCommand = new DescribeSecurityGroupsCommand({
    Filters: [
      {
        Name: 'group-name',
        Values: ['tonk-load-test-*'],
      },
    ],
  });

  const securityGroups = await ec2.send(sgCommand);
  const groupIds =
    securityGroups.SecurityGroups?.map(sg => sg.GroupId).filter(Boolean) || [];

  if (groupIds.length > 0) {
    console.log(`Found ${groupIds.length} security groups:`);
    groupIds.forEach(id => console.log(`  - ${id}`));

    for (const groupId of groupIds) {
      try {
        console.log(`Deleting security group ${groupId}...`);
        const deleteCommand = new DeleteSecurityGroupCommand({
          GroupId: groupId as string,
        });
        await ec2.send(deleteCommand);
        console.log(`✓ Deleted ${groupId}`);
      } catch (error: any) {
        console.warn(`Could not delete ${groupId}: ${error.message}`);
      }
    }
  } else {
    console.log('No test security groups found');
  }

  console.log('\n✅ Cleanup complete!');
}

cleanup().catch(error => {
  console.error('Cleanup failed:', error);
  process.exit(1);
});
