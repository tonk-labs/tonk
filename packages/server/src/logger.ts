import chalk from 'chalk';

export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

class Logger {
  private static instance: Logger;
  private logLevel: LogLevel = 'debug';

  private constructor() {}

  static getInstance(): Logger {
    if (!Logger.instance) {
      Logger.instance = new Logger();
    }
    return Logger.instance;
  }

  setLogLevel(level: LogLevel) {
    this.logLevel = level;
  }

  private getTimestamp(): string {
    return new Date().toISOString();
  }

  private shouldLog(level: LogLevel): boolean {
    const levels: LogLevel[] = ['debug', 'info', 'warn', 'error'];
    return levels.indexOf(level) >= levels.indexOf(this.logLevel);
  }

  debug(message: string, ...args: any[]) {
    if (this.shouldLog('debug')) {
      console.log(
        chalk.gray(`[${this.getTimestamp()}] DEBUG:`),
        message,
        ...args,
      );
    }
  }

  info(message: string, ...args: any[]) {
    if (this.shouldLog('info')) {
      console.log(
        chalk.blue(`[${this.getTimestamp()}] INFO:`),
        message,
        ...args,
      );
    }
  }

  warn(message: string, ...args: any[]) {
    if (this.shouldLog('warn')) {
      console.log(
        chalk.yellow(`[${this.getTimestamp()}] WARN:`),
        message,
        ...args,
      );
    }
  }

  error(message: string | Error, ...args: any[]) {
    if (this.shouldLog('error')) {
      const errorMessage =
        message instanceof Error ? message.stack || message.message : message;
      console.error(
        chalk.red(`[${this.getTimestamp()}] ERROR:`),
        errorMessage,
        ...args,
      );
    }
  }

  // Add a method to format objects for better logging
  private formatObject(obj: any): any {
    if (obj instanceof Uint8Array) {
      return `Uint8Array(${obj.length})`;
    }
    return obj;
  }

  debugWithContext(context: string, message: string, ...args: any[]) {
    if (this.shouldLog('debug')) {
      const formattedArgs = args.map(arg => this.formatObject(arg));
      console.log(
        chalk.gray(`[${this.getTimestamp()}] DEBUG [${context}]:`),
        message,
        ...formattedArgs,
      );
    }
  }
}

// Export a singleton instance
export const logger = Logger.getInstance();
