import {Command} from 'commander';
import chalk from 'chalk';
import {marked, Tokens} from 'marked';
import fs from 'fs';
import path from 'path';

interface RendererThis {
  parser: {
    parseInline: (tokens: Tokens.Generic[]) => string;
  };
}

// Create custom renderer extension
const renderer = {
  // Block-level renderer methods
  heading(this: RendererThis, token: Tokens.Heading): string {
    const prefix = '#'.repeat(token.depth);
    return chalk.bold.cyan(
      `\n${prefix} ${this.parser.parseInline(token.tokens)}\n\n`,
    );
  },
  paragraph(this: RendererThis, token: Tokens.Paragraph): string {
    return `${this.parser.parseInline(token.tokens)}\n\n`;
  },
  list(this: RendererThis, token: Tokens.List): string {
    const items = token.items.map((item, index) =>
      chalk.yellow(
        `${token.ordered ? `${index + 1}.` : '•'} ${this.parser.parseInline(
          item.tokens,
        )}`,
      ),
    );
    return items.join('\n') + '\n';
  },
  code(token: Tokens.Code): string {
    return chalk.greenBright(token.text) + '\n\n';
  },
  codespan(token: Tokens.Codespan): string {
    return chalk.greenBright(token.text);
  },
  // Inline-level renderer methods
  link(token: Tokens.Link): string {
    return chalk.blue.underline(token.href);
  },
  em(this: RendererThis, token: Tokens.Em): string {
    return chalk.italic(this.parser.parseInline(token.tokens));
  },
  strong(this: RendererThis, token: Tokens.Strong): string {
    return chalk.bold(this.parser.parseInline(token.tokens));
  },
};
// Configure marked with our custom renderer
marked.use({renderer});

export const printMarkdown = new Command('markdown')
  .description('Pretty prints markdown files')
  .argument('[path to markdown file]', 'Path of the markdown file to print')
  .action(markdownPath => {
    // Convert markdown to formatted text
    const contents = fs
      .readFileSync(path.normalize(markdownPath))
      .toString('utf8');
    const formattedMessage = marked.parse(contents);
    console.log(formattedMessage);
    process.exit(0);
  });
