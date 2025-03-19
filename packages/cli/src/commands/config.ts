import {Command} from 'commander';
import path from 'path';
import fs from 'fs';
import {execSync} from 'child_process';
import chalk from 'chalk';
import inquirer from 'inquirer';

export const configCommand = new Command('config')
  .description(
    'Configure EC2 server and create a config file for your Tonk app',
  )
  .option('-i, --instance <address>', 'EC2 instance address')
  .option('-k, --key <path>', 'Path to SSH key file')
  .option('-u, --user <name>', 'SSH username', 'ec2-user')
  .option('-p, --provision', 'Provision a new EC2 instance')
  .option('-n, --key-name <name>', 'AWS key pair name (for provisioning)')
  .action(async options => {
    const currentFileUrl = import.meta.url;
    const currentFilePath = new URL(currentFileUrl).pathname;
    const cliPath = path.dirname(path.dirname(currentFilePath));
    const awsScriptsPath = path.join(cliPath, 'scripts', 'aws');

    // Check if AWS CLI is installed
    const checkAwsInstalled = () => {
      try {
        execSync('aws --version', {stdio: 'ignore'});
        return true;
      } catch (error) {
        return false;
      }
    };

    try {
      // If provisioning or any AWS operation is needed, check for AWS CLI first
      if (options.provision) {
        if (!checkAwsInstalled()) {
          console.log(
            chalk.yellow('AWS CLI is not installed or not in your PATH.'),
          );

          const {installAws} = await inquirer.prompt([
            {
              type: 'confirm',
              name: 'installAws',
              message:
                'AWS CLI is required for provisioning. Would you like instructions to install it?',
              default: true,
            },
          ]);

          if (installAws) {
            console.log(chalk.blue('\nAWS CLI Installation Instructions:'));
            console.log(chalk.white('\nFor macOS:'));
            console.log('  brew install awscli');
            console.log('\nFor Linux:');
            console.log('  sudo apt-get install awscli  # Debian/Ubuntu');
            console.log(
              '  sudo yum install awscli      # Amazon Linux/RHEL/CentOS',
            );
            console.log('\nFor Windows:');
            console.log(
              '  Download and run the installer from: https://aws.amazon.com/cli/',
            );
            console.log('\nAfter installation, configure AWS CLI by running:');
            console.log('  aws configure\n');

            throw new Error(
              'Please install AWS CLI and run this command again.',
            );
          } else {
            throw new Error('AWS CLI is required for provisioning. Exiting.');
          }
        }

        // Check if AWS credentials are configured (account for SSO)
        try {
          // First check if any profile is available - handles both standard and SSO profiles
          const {profileName} = await inquirer.prompt([
            {
              type: 'input',
              name: 'profileName',
              message: 'Enter your AWS profile name (leave empty for default):',
              default: '',
            },
          ]);

          // Store the profile for later use
          options.awsProfile = profileName;
          const profileOption = profileName ? `--profile ${profileName}` : '';

          // Prompt for AWS region
          let region;
          try {
            // Try to get the region from the profile
            const getRegionCommand = `aws configure get region ${profileOption}`;
            region = execSync(getRegionCommand, {encoding: 'utf8'}).trim();
          } catch (error) {
            region = ''; // Region not found in profile
          }

          // If region is not set in the profile, prompt for it
          if (!region) {
            const {selectedRegion} = await inquirer.prompt([
              {
                type: 'list',
                name: 'selectedRegion',
                message: 'Select your AWS region:',
                choices: [
                  'us-east-1',
                  'us-east-2',
                  'us-west-1',
                  'us-west-2',
                  'eu-west-1',
                  'eu-west-2',
                  'eu-central-1',
                  'ap-northeast-1',
                  'ap-northeast-2',
                  'ap-southeast-1',
                  'ap-southeast-2',
                  'sa-east-1',
                ],
                default: 'us-east-1',
              },
            ]);
            region = selectedRegion;
          }

          console.log(chalk.blue(`Using AWS region: ${region}`));
          options.awsRegion = region;

          try {
            // Try to execute an AWS command with the provided profile and region
            execSync(
              `aws sts get-caller-identity ${profileOption} --region ${region}`,
              {stdio: 'ignore'},
            );
            console.log(chalk.green('AWS credentials verified successfully.'));
          } catch (error) {
            // If SSO authentication is needed
            const {useSso} = await inquirer.prompt([
              {
                type: 'confirm',
                name: 'useSso',
                message: 'Are you using AWS SSO for authentication?',
                default: false,
              },
            ]);

            if (useSso) {
              console.log(chalk.blue('\nPlease login to AWS SSO:'));
              try {
                execSync(`aws sso login ${profileOption}`, {stdio: 'inherit'});
                // Verify the login worked
                execSync(
                  `aws sts get-caller-identity ${profileOption} --region ${region}`,
                  {stdio: 'ignore'},
                );
                console.log(chalk.green('AWS SSO login successful.'));
              } catch (ssoError) {
                console.log(chalk.red('AWS SSO login failed.'));
                throw new Error(
                  'Please set up your AWS SSO profile and try again.',
                );
              }
            } else {
              console.log(
                chalk.yellow(
                  'AWS CLI is not configured with valid credentials.',
                ),
              );
              console.log(
                chalk.blue(
                  '\nPlease run the following command to configure AWS CLI:',
                ),
              );
              console.log('  aws configure\n');
              throw new Error(
                'Please configure AWS CLI and run this command again.',
              );
            }
          }
        } catch (error) {
          throw error;
        }

        if (!options.keyName) {
          const answers = await inquirer.prompt([
            {
              type: 'input',
              name: 'keyName',
              message: 'Enter a name for the AWS key pair:',
              default: 'tonk-key',
              validate: input =>
                input.length > 0 || 'Key pair name is required',
            },
          ]);
          options.keyName = answers.keyName;
        }

        console.log(chalk.blue(`Creating key pair ${options.keyName}...`));
        const keyPath = path.join(process.cwd(), `${options.keyName}.pem`);
        const profileOption = options.awsProfile
          ? `--profile ${options.awsProfile}`
          : '';
        const regionOption = options.awsRegion
          ? `--region ${options.awsRegion}`
          : '';

        // Modify the create-key.sh script call to pass region
        execSync(
          `${path.join(awsScriptsPath, 'create-key.sh')} ${options.keyName} "${profileOption}" "${regionOption}"`,
          {
            stdio: 'inherit',
          },
        );

        console.log(
          chalk.blue(
            `Provisioning EC2 instance with key ${options.keyName}...`,
          ),
        );

        // Modify the provision.sh script call to pass region
        const provisionOutput = execSync(
          `${path.join(awsScriptsPath, 'provision.sh')} ${options.keyName} "${profileOption}" "${regionOption}"`,
          {encoding: 'utf8'},
        );

        // Extract instance ID from provision output
        const instanceIdMatch = provisionOutput.match(
          /Instance launched with ID: (i-[a-z0-9]+)/,
        );
        if (!instanceIdMatch) {
          throw new Error(
            'Failed to extract instance ID from provisioning output',
          );
        }

        const instanceId = instanceIdMatch[1];
        console.log(
          chalk.blue(`Waiting for instance ${instanceId} to initialize...`),
        );

        // Wait for instance to be running and pass status checks
        execSync(
          `aws ec2 wait instance-status-ok --instance-ids ${instanceId} ${profileOption} ${regionOption}`,
          {
            stdio: 'inherit',
          },
        );

        // Get the public DNS name
        const publicDns = execSync(
          `${path.join(awsScriptsPath, 'ec2-dns.sh')} ${instanceId} "${profileOption}" "${regionOption}"`,
          {encoding: 'utf8'},
        ).trim();

        if (!publicDns) {
          throw new Error('Failed to get public DNS for the instance');
        }

        options.instance = publicDns;
        options.key = keyPath;

        console.log(
          chalk.green(`EC2 instance provisioned successfully: ${publicDns}`),
        );
      }

      // If options are not provided, prompt for them
      if (!options.instance) {
        const answers = await inquirer.prompt([
          {
            type: 'input',
            name: 'instance',
            message: 'Enter your EC2 instance address:',
            validate: input =>
              input.length > 0 || 'EC2 instance address is required',
          },
        ]);
        options.instance = answers.instance;
      }

      if (!options.key) {
        const answers = await inquirer.prompt([
          {
            type: 'input',
            name: 'key',
            message: 'Enter the path to your SSH key file:',
            validate: input => {
              if (!input.length) return 'SSH key file path is required';
              if (!fs.existsSync(input)) return 'SSH key file does not exist';
              return true;
            },
          },
        ]);
        options.key = answers.key;
      }

      // Create config file with EC2 instance URL
      const projectRoot = process.cwd();
      const configFilePath = path.join(projectRoot, 'tonk.config.json');
      const wsProtocol = 'ws';
      const wsUrl = `${wsProtocol}://${options.instance}`;

      const configData = {
        ec2: {
          instance: options.instance,
          user: options.user,
          keyPath: options.key,
        },
        websocket: {
          url: wsUrl,
        },
      };

      fs.writeFileSync(configFilePath, JSON.stringify(configData, null, 2));

      console.log(chalk.green(`Created config file at ${configFilePath}`));

      // Provision the EC2 server with basic requirements
      console.log(chalk.blue(`Setting up EC2 instance ${options.instance}...`));

      // Create a shell script with the commands to run on the EC2 instance
      const setupScript = `
# Update system packages
sudo yum update -y

# Install Docker if not already installed
if ! command -v docker &> /dev/null; then
  sudo yum install -y docker
  sudo systemctl enable docker
  sudo systemctl start docker
  sudo usermod -aG docker ${options.user}
fi

# Install Nginx for Amazon Linux 2023
sudo dnf install -y nginx

# Create necessary directories
sudo mkdir -p /etc/nginx/conf.d

# Enable and start Nginx
sudo systemctl enable nginx
sudo systemctl start nginx

# Create directory for Tonk app if it doesn't exist
mkdir -p ~/tonk-app

# Create Nginx configuration for the app
cat > /tmp/tonk-app.conf << 'EOL'
server {
    listen 80;
    server_name _;

    location / {
        proxy_pass http://localhost:${options.port || '8080'};
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
EOL

# Move Nginx config to proper location
sudo mv /tmp/tonk-app.conf /etc/nginx/conf.d/

# Make sure Nginx config is valid
echo "Testing Nginx configuration..."
sudo nginx -t || echo "Warning: Nginx configuration test failed, but continuing..."

# Set proper ownership for Amazon Linux 2023
sudo mkdir -p /var/log/nginx
sudo chown -R root:root /var/log/nginx

# Restart Nginx
sudo systemctl restart nginx

echo "Nginx configured as reverse proxy to port ${options.port || '8080'}"
`;

      // Create a temporary script and execute it
      const tempScriptPath = path.join(process.cwd(), 'tonk-setup-temp.sh');
      fs.writeFileSync(tempScriptPath, setupScript);

      try {
        // Make the script executable
        fs.chmodSync(tempScriptPath, '755');

        // Copy the script to the EC2 instance and execute it
        execSync(
          `scp -i "${options.key}" ${tempScriptPath} ${options.user}@${options.instance}:~/tonk-setup.sh && ` +
            `ssh -i "${options.key}" ${options.user}@${options.instance} "chmod +x ~/tonk-setup.sh && ~/tonk-setup.sh"`,
          {stdio: 'inherit'},
        );
      } finally {
        // Clean up the temporary script
        if (fs.existsSync(tempScriptPath)) {
          fs.unlinkSync(tempScriptPath);
        }
      }

      console.log(
        chalk.green(`
        ✅ Configuration successful!
        
        Your EC2 instance has been provisioned with Docker.
        Config file created at: ${configFilePath}
        WebSocket URL: ${wsUrl}
        
        You can now use 'tonk deploy' to deploy your app to this instance.
      `),
      );
    } catch (error) {
      console.error(chalk.red('Configuration failed:'), error);
      process.exit(1);
    }
  });
