import { Message, NetworkAdapter, PeerId } from '@automerge/automerge-repo';

export const pause = (t = 0) =>
  new Promise<void>(resolve => setTimeout(() => resolve(), t));

export class DummyNetworkAdapter extends NetworkAdapter {
  #startReady: boolean;
  #sendMessage?: SendMessageFn;
  #ready: boolean = false;

  constructor(opts: Options = { startReady: true }) {
    super();
    this.#startReady = opts.startReady || false;
    this.#sendMessage = opts.sendMessage;
  }

  isReady(): boolean {
    return this.#ready;
  }

  async whenReady(): Promise<void> {
    if (this.#ready) {
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      // For now, just resolve immediately since we're not emitting events
      resolve();
    });
  }

  connect(peerId: PeerId) {
    this.peerId = peerId;
    if (this.#startReady) {
      this.#ready = true;
      // Note: Event emission might need to be updated for v2.2.0
      // this.emit('ready', {network: this});
    }
  }

  disconnect() { }

  peerCandidate(peerId: PeerId) {
    this.emit('peer-candidate', { peerId, peerMetadata: {} });
  }

  override send(message: Message) {
    this.#sendMessage?.(message);
  }

  receive(message: Message) {
    this.emit('message', message);
  }

  static createConnectedPair({ latency = 10 }: { latency?: number } = {}) {
    const adapter1: DummyNetworkAdapter = new DummyNetworkAdapter({
      startReady: true,
      sendMessage: (message: Message) =>
        pause(latency).then(() => adapter2.receive(message)),
    });
    const adapter2: DummyNetworkAdapter = new DummyNetworkAdapter({
      startReady: true,
      sendMessage: (message: Message) =>
        pause(latency).then(() => adapter1.receive(message)),
    });

    return [adapter1, adapter2];
  }
}

type SendMessageFn = (message: Message) => void;

type Options = {
  startReady?: boolean;
  sendMessage?: SendMessageFn;
};
