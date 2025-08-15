import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { IrohNetworkAdapter } from '../../src/adapters/iroh.js';
import {
  installMockTauri,
  uninstallMockTauri,
  MockTauriAPI,
} from '../../src/adapters/mock-tauri.js';
import { PeerId, Message } from '@automerge/automerge-repo';

describe('IrohNetworkAdapter', () => {
  let adapter: IrohNetworkAdapter;
  let mockTauri: MockTauriAPI;

  beforeEach(async () => {
    // Install mock Tauri API
    mockTauri = installMockTauri();

    // Create adapter and wait for it to initialize
    adapter = new IrohNetworkAdapter();

    // Give time for the adapter to set up listeners
    await new Promise(resolve => setTimeout(resolve, 50));
  });

  afterEach(() => {
    // Clean up
    adapter.disconnect();
    uninstallMockTauri();
  });

  describe('initialization', () => {
    it('should create an adapter instance', () => {
      expect(adapter).toBeInstanceOf(IrohNetworkAdapter);
    });

    it('should not be ready initially', () => {
      expect(adapter.isReady()).toBe(false);
    });
  });

  describe('connect', () => {
    it('should set the peer ID', () => {
      const peerId = 'test-peer-id' as PeerId;
      adapter.connect(peerId);
      expect(adapter.peerId).toBe(peerId);
    });

    it('should initialize Iroh when in Tauri environment', async () => {
      const peerId = 'test-peer-id' as PeerId;
      const invokeSpy = vi.spyOn((window as any).__TAURI__, 'invoke');

      adapter.connect(peerId);

      // Wait for async initialization
      await new Promise(resolve => setTimeout(resolve, 50));

      expect(invokeSpy).toHaveBeenCalledWith('init_iroh', { peerId });
    });
  });

  describe('peer discovery', () => {
    it('should start discovery for a bundle', async () => {
      const bundleId = 'test-bundle-123';
      const invokeSpy = vi.spyOn((window as any).__TAURI__, 'invoke');

      await adapter.startDiscovery(bundleId);

      expect(invokeSpy).toHaveBeenCalledWith('start_discovery', { bundleId });
    });

    it('should handle peer discovery events', async () => {
      const peerCandidateHandler = vi.fn();
      adapter.on('peer-candidate', peerCandidateHandler);

      // Simulate peer discovery
      mockTauri.simulatePeerDiscovery('discovered-peer-1', 'bundle-123');

      // Wait for event processing
      await new Promise(resolve => setTimeout(resolve, 50));

      expect(peerCandidateHandler).toHaveBeenCalledWith(
        expect.objectContaining({
          peerId: 'discovered-peer-1',
          peerMetadata: expect.objectContaining({
            isEphemeral: false,
          }),
        })
      );
    });

    it('should auto-connect to discovered peers', async () => {
      const invokeSpy = vi.spyOn((window as any).__TAURI__, 'invoke');

      // Clear previous calls from initialization
      invokeSpy.mockClear();

      // Simulate peer discovery
      mockTauri.simulatePeerDiscovery('discovered-peer-2', 'bundle-456');

      // Wait for event processing and auto-connect
      await new Promise(resolve => setTimeout(resolve, 100));

      expect(invokeSpy).toHaveBeenCalledWith('connect_to_peer', {
        peerId: 'discovered-peer-2',
      });
    });
  });

  describe('peer connections', () => {
    it('should handle peer connection events', async () => {
      const peerHandler = vi.fn();
      adapter.on('peer-candidate', peerHandler);

      // Simulate peer discovery (which triggers peer-candidate event)
      mockTauri.simulatePeerDiscovery('connected-peer-1', 'test-bundle');

      // Wait for event processing
      await new Promise(resolve => setTimeout(resolve, 50));

      expect(peerHandler).toHaveBeenCalledWith(
        expect.objectContaining({
          peerId: 'connected-peer-1',
          peerMetadata: expect.any(Object),
        })
      );
    });

    it('should track connected peers', async () => {
      // Initially no peers
      expect(adapter.getConnectedPeers()).toHaveLength(0);

      // Simulate peer connections
      mockTauri.emitEvent('peer_connected', { peerId: 'peer-1' });
      mockTauri.emitEvent('peer_connected', { peerId: 'peer-2' });

      // Wait for event processing
      await new Promise(resolve => setTimeout(resolve, 50));

      const connectedPeers = adapter.getConnectedPeers();
      expect(connectedPeers).toContain('peer-1' as PeerId);
      expect(connectedPeers).toContain('peer-2' as PeerId);
      expect(connectedPeers).toHaveLength(2);
    });

    it('should handle peer disconnection', async () => {
      const disconnectHandler = vi.fn();
      adapter.on('peer-disconnected', disconnectHandler);

      // Connect then disconnect a peer
      mockTauri.emitEvent('peer_connected', { peerId: 'temp-peer' });
      await new Promise(resolve => setTimeout(resolve, 50));

      mockTauri.emitEvent('peer_disconnected', { peerId: 'temp-peer' });
      await new Promise(resolve => setTimeout(resolve, 50));

      expect(disconnectHandler).toHaveBeenCalledWith(
        expect.objectContaining({
          peerId: 'temp-peer',
        })
      );

      expect(adapter.getConnectedPeers()).not.toContain('temp-peer' as PeerId);
    });
  });

  describe('message handling', () => {
    it('should send messages to peers', async () => {
      const invokeSpy = vi.spyOn((window as any).__TAURI__, 'invoke');

      // Clear previous calls from initialization
      invokeSpy.mockClear();

      const message: Message = {
        senderId: 'sender-1' as PeerId,
        targetId: 'target-1' as PeerId,
        type: 'sync',
        data: new Uint8Array([1, 2, 3]),
      };

      adapter.send(message);

      // Wait for async send (if any)
      await new Promise(resolve => setTimeout(resolve, 50));

      expect(invokeSpy).toHaveBeenCalledWith('send_automerge_message', {
        targetId: 'target-1',
        message: {
          type: 'sync',
          data: [1, 2, 3], // Uint8Array converted to array
        },
      });
    });

    it('should receive messages from peers', async () => {
      const messageHandler = vi.fn();
      adapter.on('message', messageHandler);

      // Simulate receiving a message
      mockTauri.simulateMessage('remote-peer', {
        type: 'sync',
        data: [4, 5, 6],
      });

      // Wait for event processing
      await new Promise(resolve => setTimeout(resolve, 50));

      expect(messageHandler).toHaveBeenCalledWith(
        expect.objectContaining({
          senderId: 'remote-peer',
          type: 'sync',
          data: expect.any(Uint8Array),
        })
      );

      // Check data conversion
      const receivedMessage = messageHandler.mock.calls[0][0];
      expect(Array.from(receivedMessage.data)).toEqual([4, 5, 6]);
    });

    it('should not send messages without target ID', () => {
      const invokeSpy = vi.spyOn((window as any).__TAURI__, 'invoke');

      // Clear previous calls
      invokeSpy.mockClear();

      const message: Message = {
        senderId: 'sender-2' as PeerId,
        targetId: undefined as any,
        type: 'sync',
        data: new Uint8Array([7, 8, 9]),
      };

      adapter.send(message);

      expect(invokeSpy).not.toHaveBeenCalledWith(
        'send_automerge_message',
        expect.anything()
      );
    });
  });

  describe('network readiness', () => {
    it('should become ready when Iroh is ready', async () => {
      // Wait for the mock ready event (emitted after 100ms in mock)
      await adapter.whenReady();

      expect(adapter.isReady()).toBe(true);
    });
  });

  describe('disconnect', () => {
    it('should disconnect from all peers', async () => {
      const invokeSpy = vi.spyOn((window as any).__TAURI__, 'invoke');

      // Connect some peers first
      mockTauri.emitEvent('peer_connected', { peerId: 'peer-a' });
      mockTauri.emitEvent('peer_connected', { peerId: 'peer-b' });
      await new Promise(resolve => setTimeout(resolve, 50));

      // Clear previous calls
      invokeSpy.mockClear();

      adapter.disconnect();

      expect(invokeSpy).toHaveBeenCalledWith('disconnect_all_peers');
      expect(adapter.getConnectedPeers()).toHaveLength(0);
      expect(adapter.isReady()).toBe(false);
    });
  });
});

describe('IrohNetworkAdapter without Tauri', () => {
  let adapter: IrohNetworkAdapter;

  beforeEach(async () => {
    // Ensure no Tauri environment
    if (typeof window !== 'undefined') {
      delete (window as any).__TAURI__;
    }
    adapter = new IrohNetworkAdapter();

    // Wait for initialization
    await new Promise(resolve => setTimeout(resolve, 50));
  });

  afterEach(() => {
    adapter.disconnect();
  });

  it('should be ready immediately when not in Tauri environment', async () => {
    // Should already be ready from initialization
    expect(adapter.isReady()).toBe(true);

    adapter.connect('test-peer' as PeerId);

    // Verify still ready after connect
    expect(adapter.isReady()).toBe(true);
  });

  it('should not crash when calling methods outside Tauri', async () => {
    // These should not throw errors
    expect(() => adapter.send({} as Message)).not.toThrow();
    expect(() => adapter.startDiscovery('bundle-123')).not.toThrow();
    expect(() => adapter.disconnect()).not.toThrow();
  });
});
