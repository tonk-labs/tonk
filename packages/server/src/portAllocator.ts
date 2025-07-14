export class PortAllocator {
  private usedPorts: Set<number> = new Set();
  private startingPort = 6081;

  /**
   * Allocate the next available port starting from 6081
   */
  async allocate(): Promise<number> {
    let port = this.startingPort;
    while (this.usedPorts.has(port)) {
      port++;
    }
    this.usedPorts.add(port);
    return port;
  }

  /**
   * Deallocate a port, making it available for reuse
   */
  async deallocate(port: number): Promise<void> {
    this.usedPorts.delete(port);
  }

  /**
   * Check if a port is currently allocated
   */
  isAllocated(port: number): boolean {
    return this.usedPorts.has(port);
  }

  /**
   * Get list of all allocated ports
   */
  getAllocatedPorts(): number[] {
    return Array.from(this.usedPorts).sort((a, b) => a - b);
  }
}
