import { writeDoc, readDoc, rm } from "@tonk/keepsync";
import { TextContent, ImageContent, LinkContent } from "../types/posts";

export type ContentType = TextContent | ImageContent | LinkContent;

export class ContentStorage {
  private static getContentPath(contentId: string): string {
    return `/content/${contentId}`;
  }

  static async saveTextContent(
    contentId: string,
    content: TextContent,
  ): Promise<void> {
    const path = this.getContentPath(contentId);
    await writeDoc(path, content);
  }

  static async saveImageContent(
    contentId: string,
    content: ImageContent,
  ): Promise<void> {
    const path = this.getContentPath(contentId);
    await writeDoc(path, content);
  }

  static async saveLinkContent(
    contentId: string,
    content: LinkContent,
  ): Promise<void> {
    const path = this.getContentPath(contentId);
    await writeDoc(path, content);
  }

  static async loadTextContent(contentId: string): Promise<TextContent | null> {
    const path = this.getContentPath(contentId);
    const content = await readDoc<TextContent>(path);
    return content || null;
  }

  static async loadImageContent(
    contentId: string,
  ): Promise<ImageContent | null> {
    const path = this.getContentPath(contentId);
    const content = await readDoc<ImageContent>(path);
    return content || null;
  }

  static async loadLinkContent(contentId: string): Promise<LinkContent | null> {
    const path = this.getContentPath(contentId);
    const content = await readDoc<LinkContent>(path);
    return content || null;
  }

  static async deleteContent(contentId: string): Promise<void> {
    const path = this.getContentPath(contentId);
    try {
      await rm(path);
    } catch (error) {
      console.warn(`Failed to delete content ${contentId}:`, error);
    }
  }

  static generateContentId(): string {
    return `content_${crypto.randomUUID()}`;
  }
}
