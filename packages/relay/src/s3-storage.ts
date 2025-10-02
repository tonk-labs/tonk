import {
  S3Client,
  GetObjectCommand,
  HeadObjectCommand,
  ListObjectsCommand,
} from '@aws-sdk/client-s3';
import { Upload } from '@aws-sdk/lib-storage';

export interface S3Config {
  region: string;
  bucket: string;
}

export class S3Storage {
  private client: S3Client;
  private bucket: string;
  private isAvailable = false;

  constructor(config: S3Config) {
    this.bucket = config.bucket;

    const clientConfig: any = {
      region: config.region,
    };

    this.client = new S3Client(clientConfig);
  }

  async uploadBundle(bundleId: string, data: Buffer): Promise<void> {
    if (!this.healthCheck()) return;
    const key = `bundles/${bundleId}.tonk`;

    try {
      const upload = new Upload({
        client: this.client,
        params: {
          Bucket: this.bucket,
          Key: key,
          Body: data,
          ContentType: 'application/octet-stream',
          Metadata: {
            uploadedAt: new Date().toISOString(),
          },
        },
      });

      await upload.done();
      console.log(`Bundle uploaded successfully: ${key}`);
    } catch (error) {
      console.error(`Failed to upload bundle ${bundleId}:`, error);
      throw new Error(`Failed to upload bundle: ${error}`);
    }
  }

  async downloadBundle(bundleId: string): Promise<Buffer | undefined> {
    if (!this.healthCheck()) return;
    const key = `bundles/${bundleId}.tonk`;

    try {
      const command = new GetObjectCommand({
        Bucket: this.bucket,
        Key: key,
      });

      const response = await this.client.send(command);

      if (!response.Body) {
        throw new Error('No data received from S3');
      }

      const chunks: Uint8Array[] = [];
      for await (const chunk of response.Body as any) {
        chunks.push(chunk);
      }

      return Buffer.concat(chunks);
    } catch (error: any) {
      if (error.name === 'NoSuchKey') {
        throw new Error(`Bundle not found: ${bundleId}`);
      }
      console.error(`Failed to download bundle ${bundleId}:`, error);
      throw new Error(`Failed to download bundle: ${error}`);
    }
  }

  async bundleExists(bundleId: string): Promise<boolean | undefined> {
    if (!this.healthCheck()) return;
    const key = `bundles/${bundleId}.tonk`;

    try {
      await this.client.send(
        new HeadObjectCommand({
          Bucket: this.bucket,
          Key: key,
        })
      );
      return true;
    } catch (error: any) {
      if (error.name === 'NotFound' || error.name === 'NoSuchKey') {
        return false;
      }
      throw error;
    }
  }

  async getBundleMetadata(bundleId: string): Promise<{
    size: number;
    lastModified: Date;
  } | null> {
    if (!this.healthCheck()) return null;
    const key = `bundles/${bundleId}.tonk`;

    try {
      const response = await this.client.send(
        new HeadObjectCommand({
          Bucket: this.bucket,
          Key: key,
        })
      );

      return {
        size: response.ContentLength || 0,
        lastModified: response.LastModified || new Date(),
      };
    } catch (error: any) {
      if (error.name === 'NotFound' || error.name === 'NoSuchKey') {
        return null;
      }
      throw error;
    }
  }

  async healthCheck(): Promise<boolean> {
    if (!this.isAvailable) {
      try {
        await this.client.send(
          new ListObjectsCommand({ Bucket: this.bucket, MaxKeys: 1 })
        );
        this.isAvailable = true;
        return true;
      } catch (error: any) {
        console.error('S3 not available:', error.message);
        return false;
      }
    }

    return true;
  }
}
