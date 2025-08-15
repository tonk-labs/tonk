import {
  Message,
  NetworkAdapter,
  PeerId,
  PeerMetadata,
} from '@automerge/automerge-repo';

export const pause = (t = 0) =>
  new Promise<void>(resolve => setTimeout(() => resolve(), t));

export class DummyNetworkAdapter extends NetworkAdapter {
  #startReady: boolean;
  #sendMessage?: SendMessageFn;
  #ready: boolean = false;
  #readyPromise: Promise<void>;
  #readyResolve: ((value: void) => void) | null = null;

  constructor(opts: Options = { startReady: true }) {
    super();
    this.#startReady = opts.startReady || false;
    this.#sendMessage = opts.sendMessage;
    this.#readyPromise = new Promise<void>(resolve => {
      this.#readyResolve = resolve;
    });
  }

  isReady(): boolean {
    return this.#ready;
  }

  whenReady(): Promise<void> {
    return this.#readyPromise;
  }

  connect(peerId: PeerId, peerMetadata?: PeerMetadata) {
    this.peerId = peerId;
    this.peerMetadata = peerMetadata;
    if (this.#startReady) {
      this.#ready = true;
      if (this.#readyResolve) {
        this.#readyResolve();
      }
    }
  }

  disconnect() {
    this.#ready = false;
  }

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
