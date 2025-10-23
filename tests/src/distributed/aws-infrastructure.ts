import {
  EC2Client,
  RunInstancesCommand,
  TerminateInstancesCommand,
  DescribeInstancesCommand,
  CreateSecurityGroupCommand,
  AuthorizeSecurityGroupIngressCommand,
  DeleteSecurityGroupCommand,
  CreateKeyPairCommand,
  DeleteKeyPairCommand,
  DescribeKeyPairsCommand,
  DescribeSecurityGroupsCommand,
} from '@aws-sdk/client-ec2';
import { promises as fs } from 'fs';
import { homedir } from 'os';
import { join } from 'path';
import type { EC2Config, ProvisionedInstance } from './types';

export class AWSInfrastructure {
  private ec2Client: EC2Client;
  private config: EC2Config;
  private securityGroupId?: string;
  private keyPath?: string;

  constructor(config: EC2Config) {
    this.config = config;
    this.ec2Client = new EC2Client({ region: config.region });
  }

  async setupKeyPair(): Promise<string> {
    console.log(`Setting up SSH key pair: ${this.config.keyName}`);

    try {
      const keyPath = join(homedir(), '.ssh', `${this.config.keyName}.pem`);

      const localKeyExists = await fs
        .access(keyPath)
        .then(() => true)
        .catch(() => false);

      const describeKeyPairCommand = new DescribeKeyPairsCommand({
        KeyNames: [this.config.keyName],
      });

      let awsKeyExists = false;
      try {
        await this.ec2Client.send(describeKeyPairCommand);
        awsKeyExists = true;
      } catch (error: any) {
        if (error.name !== 'InvalidKeyPair.NotFound') {
          throw error;
        }
      }

      if (awsKeyExists && localKeyExists) {
        console.log(
          `✓ Key pair already exists in AWS and locally at ${keyPath}`
        );
        this.keyPath = keyPath;
        return keyPath;
      }

      if (awsKeyExists && !localKeyExists) {
        console.log(
          'Key pair exists in AWS but not locally. Deleting from AWS and recreating...'
        );
        const deleteKeyPairCommand = new DeleteKeyPairCommand({
          KeyName: this.config.keyName,
        });
        await this.ec2Client.send(deleteKeyPairCommand);
        console.log('✓ Key pair deleted from AWS');
      }

      console.log('Creating new key pair...');
      const createKeyPairCommand = new CreateKeyPairCommand({
        KeyName: this.config.keyName,
      });

      const response = await this.ec2Client.send(createKeyPairCommand);

      if (!response.KeyMaterial) {
        throw new Error('No key material returned from AWS');
      }

      await fs.writeFile(keyPath, response.KeyMaterial, { mode: 0o400 });
      console.log(`✓ Key pair created and saved to ${keyPath}`);
      this.keyPath = keyPath;
      return keyPath;
    } catch (error) {
      throw new Error(`Failed to setup key pair: ${error}`);
    }
  }

  async setupSecurityGroup(): Promise<string> {
    console.log(`Setting up security group: ${this.config.securityGroupName}`);

    try {
      const describeCommand = new DescribeSecurityGroupsCommand({
        Filters: [
          {
            Name: 'group-name',
            Values: [this.config.securityGroupName],
          },
        ],
      });

      const existing = await this.ec2Client.send(describeCommand);

      if (existing.SecurityGroups && existing.SecurityGroups.length > 0) {
        const groupId = existing.SecurityGroups[0].GroupId;
        if (!groupId) {
          throw new Error('Security group ID not found');
        }
        console.log(`✓ Security group already exists: ${groupId}`);
        this.securityGroupId = groupId;
        return groupId;
      }

      console.log('Creating new security group...');
      const createCommand = new CreateSecurityGroupCommand({
        GroupName: this.config.securityGroupName,
        Description: 'Security group for Tonk distributed load testing workers',
      });

      const createResponse = await this.ec2Client.send(createCommand);
      const groupId = createResponse.GroupId;

      if (!groupId) {
        throw new Error('Failed to create security group');
      }

      const authorizeCommand = new AuthorizeSecurityGroupIngressCommand({
        GroupId: groupId,
        IpPermissions: [
          {
            IpProtocol: 'tcp',
            FromPort: 22,
            ToPort: 22,
            IpRanges: [{ CidrIp: '0.0.0.0/0', Description: 'SSH access' }],
          },
          {
            IpProtocol: 'tcp',
            FromPort: 3000,
            ToPort: 3000,
            IpRanges: [{ CidrIp: '0.0.0.0/0', Description: 'Worker HTTP API' }],
          },
          {
            IpProtocol: 'tcp',
            FromPort: 5173,
            ToPort: 5173,
            IpRanges: [{ CidrIp: '0.0.0.0/0', Description: 'Vite dev server' }],
          },
          {
            IpProtocol: 'tcp',
            FromPort: 443,
            ToPort: 443,
            IpRanges: [{ CidrIp: '0.0.0.0/0', Description: 'HTTPS outbound' }],
          },
          {
            IpProtocol: 'tcp',
            FromPort: 8081,
            ToPort: 8081,
            IpRanges: [{ CidrIp: '0.0.0.0/0', Description: 'Relay WebSocket' }],
          },
          {
            IpProtocol: 'tcp',
            FromPort: 9000,
            ToPort: 9000,
            IpRanges: [{ CidrIp: '0.0.0.0/0', Description: 'Coordinator API' }],
          },
        ],
      });

      await this.ec2Client.send(authorizeCommand);
      console.log(`✓ Security group created with ingress rules: ${groupId}`);

      this.securityGroupId = groupId;
      return groupId;
    } catch (error) {
      throw new Error(`Failed to setup security group: ${error}`);
    }
  }

  async provisionCoordinator(): Promise<ProvisionedInstance> {
    console.log('Provisioning coordinator instance...');

    if (!this.securityGroupId) {
      await this.setupSecurityGroup();
    }

    if (!this.keyPath) {
      await this.setupKeyPair();
    }

    const amiId =
      this.config.amiId || (await this.getLatestAmazonLinux2023AMI());

    console.log(`Using AMI: ${amiId}`);
    console.log('Instance type: t3.small');

    const runInstancesParams: any = {
      ImageId: amiId,
      InstanceType: 't3.small',
      MinCount: 1,
      MaxCount: 1,
      KeyName: this.config.keyName,
      SecurityGroupIds: [this.securityGroupId],
      TagSpecifications: [
        {
          ResourceType: 'instance',
          Tags: [
            {
              Key: 'Name',
              Value: `${this.config.instanceTags.Name}-coordinator`,
            },
            { Key: 'Role', Value: 'coordinator' },
            ...Object.entries(this.config.instanceTags)
              .filter(([key]) => key !== 'Name')
              .map(([Key, Value]) => ({ Key, Value })),
          ],
        },
      ],
    };

    try {
      const command = new RunInstancesCommand(runInstancesParams);
      const response = await this.ec2Client.send(command);

      if (!response.Instances || response.Instances.length === 0) {
        throw new Error('No coordinator instance was created');
      }

      const instanceId = response.Instances[0].InstanceId;
      if (!instanceId) {
        throw new Error('No instance ID returned');
      }

      console.log(`✓ Coordinator instance created: ${instanceId}`);
      console.log('Waiting for coordinator instance to be running...');

      await this.waitForInstances([instanceId], 'running');

      const instances = await this.describeInstances([instanceId]);
      console.log('✓ Coordinator instance is running');

      return instances[0];
    } catch (error) {
      throw new Error(`Failed to provision coordinator: ${error}`);
    }
  }

  async provisionWorkers(count: number): Promise<ProvisionedInstance[]> {
    console.log(`Provisioning ${count} worker instances...`);

    if (!this.securityGroupId) {
      await this.setupSecurityGroup();
    }

    if (!this.keyPath) {
      await this.setupKeyPair();
    }

    const amiId =
      this.config.amiId || (await this.getLatestAmazonLinux2023AMI());

    console.log(`Using AMI: ${amiId}`);
    console.log(`Instance type: ${this.config.instanceType}`);
    console.log(`Spot instances: ${this.config.useSpotInstances}`);

    const runInstancesParams: any = {
      ImageId: amiId,
      InstanceType: this.config.instanceType,
      MinCount: count,
      MaxCount: count,
      KeyName: this.config.keyName,
      SecurityGroupIds: [this.securityGroupId],
      TagSpecifications: [
        {
          ResourceType: 'instance',
          Tags: Object.entries(this.config.instanceTags).map(
            ([Key, Value]) => ({
              Key,
              Value,
            })
          ),
        },
      ],
    };

    if (this.config.useSpotInstances) {
      runInstancesParams.InstanceMarketOptions = {
        MarketType: 'spot',
        SpotOptions: {
          MaxPrice: this.config.maxSpotPrice || '0.50',
          SpotInstanceType: 'one-time',
          InstanceInterruptionBehavior: 'terminate',
        },
      };
    }

    try {
      const command = new RunInstancesCommand(runInstancesParams);
      const response = await this.ec2Client.send(command);

      if (!response.Instances || response.Instances.length === 0) {
        throw new Error('No instances were created');
      }

      const instanceIds = response.Instances.map(
        (i: any) => i.InstanceId
      ).filter(Boolean) as string[];

      console.log(`✓ Instances created: ${instanceIds.join(', ')}`);
      console.log('Waiting for instances to be running...');

      await this.waitForInstances(instanceIds, 'running');

      const instances = await this.describeInstances(instanceIds);
      console.log(`✓ All ${instances.length} instances are running`);

      return instances;
    } catch (error) {
      throw new Error(`Failed to provision workers: ${error}`);
    }
  }

  private async getLatestAmazonLinux2023AMI(): Promise<string> {
    return 'ami-0854d4f8e4bd6b834';
  }

  private async waitForInstances(
    instanceIds: string[],
    state: 'running' | 'terminated',
    maxWaitTimeMs: number = 300000
  ): Promise<void> {
    const startTime = Date.now();

    while (Date.now() - startTime < maxWaitTimeMs) {
      const command = new DescribeInstancesCommand({
        InstanceIds: instanceIds,
      });

      const response = await this.ec2Client.send(command);
      const instances =
        response.Reservations?.flatMap((r: any) => r.Instances || []) || [];

      const allInDesiredState = instances.every(
        (i: any) => i.State?.Name === state
      );

      if (allInDesiredState) {
        return;
      }

      await new Promise(resolve => setTimeout(resolve, 5000));
    }

    throw new Error(`Timeout waiting for instances to reach state: ${state}`);
  }

  async describeInstances(
    instanceIds: string[]
  ): Promise<ProvisionedInstance[]> {
    const command = new DescribeInstancesCommand({
      InstanceIds: instanceIds,
    });

    const response = await this.ec2Client.send(command);
    const instances =
      response.Reservations?.flatMap((r: any) => r.Instances || []) || [];

    return instances.map((instance: any) => ({
      instanceId: instance.InstanceId || '',
      publicIp: instance.PublicIpAddress || '',
      privateIp: instance.PrivateIpAddress || '',
      publicDns: instance.PublicDnsName || '',
      status: instance.State?.Name || 'unknown',
      launchedAt: instance.LaunchTime?.getTime() || Date.now(),
    }));
  }

  async waitForSSHReady(
    instances: ProvisionedInstance[],
    maxWaitTimeMs: number = 180000
  ): Promise<void> {
    console.log('Waiting for SSH to be ready on all instances...');
    const startTime = Date.now();

    await Promise.all(
      instances.map(async instance => {
        console.log(
          `  Checking ${instance.instanceId} (${instance.publicIp})...`
        );

        while (Date.now() - startTime < maxWaitTimeMs) {
          try {
            const { execSync } = await import('child_process');
            execSync(
              `ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 -i ${this.keyPath} ec2-user@${instance.publicIp} "echo ready"`,
              { stdio: 'pipe', timeout: 10000 }
            );
            console.log(`  ✓ ${instance.instanceId} SSH ready`);
            return;
          } catch {
            await new Promise(resolve => setTimeout(resolve, 5000));
          }
        }

        throw new Error(
          `Timeout waiting for SSH on ${instance.instanceId} (${instance.publicIp})`
        );
      })
    );

    console.log('✓ All instances have SSH ready');
  }

  async terminateInstances(instanceIds: string[]): Promise<void> {
    if (instanceIds.length === 0) {
      console.log('No instances to terminate');
      return;
    }

    console.log(`Terminating ${instanceIds.length} instances...`);

    try {
      const command = new TerminateInstancesCommand({
        InstanceIds: instanceIds,
      });

      await this.ec2Client.send(command);
      console.log(`✓ Termination initiated for: ${instanceIds.join(', ')}`);

      await this.waitForInstances(instanceIds, 'terminated', 120000);
      console.log('✓ All instances terminated');
    } catch (error) {
      console.error(`Error terminating instances: ${error}`);
      throw error;
    }
  }

  async cleanup(instanceIds: string[]): Promise<void> {
    console.log('Cleaning up AWS resources...');

    if (instanceIds.length > 0) {
      await this.terminateInstances(instanceIds);
    }

    if (
      this.securityGroupId &&
      this.config.securityGroupName.includes('test')
    ) {
      try {
        await new Promise(resolve => setTimeout(resolve, 30000));

        const command = new DeleteSecurityGroupCommand({
          GroupId: this.securityGroupId,
        });
        await this.ec2Client.send(command);
        console.log(`✓ Security group deleted: ${this.securityGroupId}`);
      } catch (error) {
        console.warn(`Could not delete security group: ${error}`);
      }
    }

    console.log('✓ Cleanup complete');
  }

  getKeyPath(): string {
    if (!this.keyPath) {
      throw new Error('Key pair not set up');
    }
    return this.keyPath;
  }

  getSecurityGroupId(): string {
    if (!this.securityGroupId) {
      throw new Error('Security group not set up');
    }
    return this.securityGroupId;
  }

  async estimateCost(
    instanceCount: number,
    durationHours: number
  ): Promise<{ perHour: number; total: number }> {
    const pricePerHour = this.config.useSpotInstances ? 0.115 : 0.384;

    return {
      perHour: pricePerHour * instanceCount,
      total: pricePerHour * instanceCount * durationHours,
    };
  }
}
