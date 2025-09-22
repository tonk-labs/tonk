export interface ComponentSignature {
  hookOrder: string[];
  hookCount: number;
  componentName: string;
  lastCompiled: number;
  checksum?: string;
}

export interface HookState {
  type: 'state' | 'ref' | 'memo' | 'callback' | 'effect';
  value: any;
  deps?: any[];
  index: number;
}

export interface ProxiedComponent {
  id: string;
  original: React.ComponentType<any>;
  proxy: React.ComponentType<any>;
  signature: ComponentSignature;
  version: number;
}

class ComponentRegistry {
  private components: Map<string, ProxiedComponent> = new Map();
  private stateCache: Map<string, HookState[]> = new Map();
  private signatures: Map<string, ComponentSignature> = new Map();
  private updateCallbacks: Map<string, Set<() => void>> = new Map();

  register(id: string, component: React.ComponentType<any>): ProxiedComponent {
    const signature = this.generateSignature(component);
    const proxied = this.createProxy(id, component, signature);

    this.components.set(id, proxied);
    this.signatures.set(id, signature);

    return proxied;
  }

  update(id: string, newComponent: React.ComponentType<any>): boolean {
    const existing = this.components.get(id);
    if (!existing) {
      return false;
    }

    const newSignature = this.generateSignature(newComponent);
    const canPreserveState = this.canPreserveState(
      existing.signature,
      newSignature
    );

    if (!canPreserveState) {
      this.stateCache.delete(id);
    }

    existing.original = newComponent;
    existing.signature = newSignature;
    existing.version++;

    this.signatures.set(id, newSignature);
    this.notifyUpdate(id);

    return canPreserveState;
  }

  getState(id: string): HookState[] | undefined {
    return this.stateCache.get(id);
  }

  preserveState(id: string, state: HookState[]): void {
    this.stateCache.set(id, state);
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

  private notifyUpdate(id: string): void {
    const callbacks = this.updateCallbacks.get(id);
    if (callbacks) {
      callbacks.forEach(cb => cb());
    }
  }

  private createProxy(
    id: string,
    component: React.ComponentType<any>,
    signature: ComponentSignature
  ): ProxiedComponent {
    const registry = this;

    const ProxyComponent = function (props: any) {
      const React = (window as any).React;
      const [, forceUpdate] = React.useState(0);
      const [error, setError] = React.useState(null);

      React.useEffect(() => {
        const unsubscribe = registry.onUpdate(id, () => {
          forceUpdate((v: number) => v + 1);
          setError(null);
        });
        return unsubscribe;
      }, []);

      if (error) {
        return React.createElement(
          'div',
          {
            style: {
              padding: '20px',
              backgroundColor: '#fee',
              border: '1px solid #fcc',
              borderRadius: '4px',
              color: '#c00',
            },
          },
          [
            React.createElement('h3', { key: 'title' }, 'Component Error'),
            React.createElement(
              'pre',
              {
                key: 'error',
                style: { marginTop: '10px', fontSize: '12px' },
              },
              (error as any).message
            ),
          ]
        );
      }

      try {
        const current = registry.components.get(id);
        const Component = current?.original || component;
        return React.createElement(Component, props);
      } catch (err) {
        setError(err as any);
        return null;
      }
    };

    ProxyComponent.displayName = `HotProxy(${component.displayName || component.name || 'Component'})`;

    return {
      id,
      original: component,
      proxy: ProxyComponent as React.ComponentType<any>,
      signature,
      version: 1,
    };
  }

  private generateSignature(
    component: React.ComponentType<any>
  ): ComponentSignature {
    const componentString = component.toString();
    const hookRegex = /use[A-Z]\w*/g;
    const hooks = componentString.match(hookRegex) || [];

    return {
      hookOrder: hooks,
      hookCount: hooks.length,
      componentName: component.name || 'Anonymous',
      lastCompiled: Date.now(),
      checksum: this.simpleChecksum(componentString),
    };
  }

  private canPreserveState(
    oldSig: ComponentSignature,
    newSig: ComponentSignature
  ): boolean {
    if (oldSig.hookCount !== newSig.hookCount) {
      return false;
    }

    for (let i = 0; i < oldSig.hookOrder.length; i++) {
      if (oldSig.hookOrder[i] !== newSig.hookOrder[i]) {
        return false;
      }
    }

    return true;
  }

  private simpleChecksum(str: string): string {
    let hash = 0;
    for (let i = 0; i < str.length; i++) {
      const char = str.charCodeAt(i);
      hash = (hash << 5) - hash + char;
      hash = hash & hash;
    }
    return hash.toString(36);
  }

  clear(): void {
    this.components.clear();
    this.stateCache.clear();
    this.signatures.clear();
    this.updateCallbacks.clear();
  }

  getComponent(id: string): ProxiedComponent | undefined {
    return this.components.get(id);
  }
}

export const componentRegistry = new ComponentRegistry();
