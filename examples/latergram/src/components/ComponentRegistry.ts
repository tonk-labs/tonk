import { createInlineErrorBoundary } from './errors/createInlineErrorBoundary';

export interface ComponentMetadata {
  id: string;
  name: string;
  filePath: string;
  created: Date;
  modified: Date;
  status: 'loading' | 'success' | 'error';
  error?: string;
}

export interface ProxiedComponent {
  id: string;
  metadata: ComponentMetadata;
  original: React.ComponentType<any>;
  proxy: React.ComponentType<any>;
  version: number;
}

class ComponentRegistry {
  private components: Map<string, ProxiedComponent> = new Map();
  private updateCallbacks: Map<string, Set<() => void>> = new Map();
  private contextUpdateCallbacks: Set<() => void> = new Set();

  register(
    id: string,
    component: React.ComponentType<any>,
    metadata?: Partial<ComponentMetadata>
  ): ProxiedComponent {
    const now = new Date();
    const componentMetadata: ComponentMetadata = {
      id,
      name: metadata?.name || `Component-${id}`,
      filePath: metadata?.filePath || `/src/components/${id}.tsx`,
      created: metadata?.created || now,
      modified: metadata?.modified || now,
      status: 'success',
      ...metadata,
    };

    const proxied = this.createProxy(id, component, componentMetadata);
    this.components.set(id, proxied);
    this.notifyContextUpdate();

    return proxied;
  }

  createComponent(name: string, filePath?: string): string {
    const id = this.generateId();
    const metadata: ComponentMetadata = {
      id,
      name,
      filePath: filePath || `/src/components/${id}.tsx`,
      created: new Date(),
      modified: new Date(),
      status: 'loading',
    };

    const dummyComponent = () => null;
    const proxied = this.createProxy(id, dummyComponent, metadata);
    this.components.set(id, proxied);
    this.notifyContextUpdate();

    return id;
  }

  updateMetadata(id: string, updates: Partial<ComponentMetadata>): void {
    const existing = this.components.get(id);
    if (existing) {
      existing.metadata = {
        ...existing.metadata,
        ...updates,
        modified: new Date(),
      };
      this.notifyUpdate(id);
    }
  }

  getAllComponents(): ProxiedComponent[] {
    return Array.from(this.components.values());
  }

  getComponentsByStatus(
    status: ComponentMetadata['status']
  ): ProxiedComponent[] {
    return this.getAllComponents().filter(
      comp => comp.metadata.status === status
    );
  }

  deleteComponent(id: string): boolean {
    const deleted = this.components.delete(id);
    if (deleted) {
      this.updateCallbacks.delete(id);
      this.notifyContextUpdate();
    }
    return deleted;
  }

  private generateId(): string {
    return `comp-${Date.now()}-${Math.random().toString(36).substring(2, 11)}`;
  }

  update(
    id: string,
    newComponent: React.ComponentType<any>,
    status: ComponentMetadata['status'] = 'success',
    error?: string
  ): void {
    const existing = this.components.get(id);
    if (!existing) {
      return;
    }

    existing.original = newComponent;
    existing.version++;
    existing.metadata.modified = new Date();
    existing.metadata.status = status;
    if (error) {
      existing.metadata.error = error;
    } else {
      delete existing.metadata.error;
    }

    this.notifyUpdate(id);
    this.notifyContextUpdate();
  }

  onUpdate(id: string, callback: () => void): () => void {
    if (!this.updateCallbacks.has(id)) {
      this.updateCallbacks.set(id, new Set());
    }

    const callbacks = this.updateCallbacks.get(id)!;
    callbacks.add(callback);

    return () => {
      callbacks.delete(callback);
      if (callbacks.size === 0) {
        this.updateCallbacks.delete(id);
      }
    };
  }

  onContextUpdate(callback: () => void): () => void {
    this.contextUpdateCallbacks.add(callback);
    return () => {
      this.contextUpdateCallbacks.delete(callback);
    };
  }

  private notifyUpdate(id: string): void {
    const callbacks = this.updateCallbacks.get(id);
    if (callbacks) {
      callbacks.forEach(cb => cb());
    }
  }

  private notifyContextUpdate(): void {
    this.contextUpdateCallbacks.forEach(cb => cb());
  }

  private createProxy(
    id: string,
    component: React.ComponentType<any>,
    metadata: ComponentMetadata
  ): ProxiedComponent {
    const registry = this;

    const ProxyComponent = function (props: any) {
      const React = (window as any).React;
      const [, forceUpdate] = React.useState(0);

      React.useEffect(() => {
        const unsubscribe = registry.onUpdate(id, () => {
          forceUpdate((v: number) => v + 1);
        });
        return unsubscribe;
      }, []);

      const current = registry.components.get(id);
      const Component = current?.original || component;

      // Use the shared error boundary creator
      const ErrorBoundaryClass = createInlineErrorBoundary(React, metadata.name);

      // Wrap the component with error boundary
      return React.createElement(
        ErrorBoundaryClass,
        {},
        React.createElement(Component, props)
      );
    };

    ProxyComponent.displayName = `HotProxy(${component.displayName || component.name || 'Component'})`;

    return {
      id,
      metadata,
      original: component,
      proxy: ProxyComponent as React.ComponentType<any>,
      version: 1,
    };
  }

  clear(): void {
    this.components.clear();
    this.updateCallbacks.clear();
    this.notifyContextUpdate();
  }

  getComponent(id: string): ProxiedComponent | undefined {
    return this.components.get(id);
  }

  getComponentByFilePath(filePath: string): ProxiedComponent | undefined {
    return Array.from(this.components.values()).find(
      comp => comp.metadata.filePath === filePath
    );
  }

  getComponentByName(name: string): ProxiedComponent | undefined {
    return Array.from(this.components.values()).find(
      comp => comp.metadata.name === name
    );
  }
}

export const componentRegistry = new ComponentRegistry();
