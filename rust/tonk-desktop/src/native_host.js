// Runs before any page script, in every frame of the window.
//
// It tells the page it is hosted natively, and it carries live queries
// over WebSockets. The page opens each live query as a `fetch` whose
// response streams for as long as the query is watched. A service worker
// answers those inside the browser, but here they are real connections
// to a loopback server, and browsers allow only six HTTP/1.1 connections
// to one host at a time: a page watching more than six queries stalls.
// WebSockets do not count against that limit, so a streaming request is
// sent as one, and its response is rebuilt as an ordinary `Response`.
(() => {
    globalThis.tonkNativeHost = Object.freeze({ kind: "desktop" });

    // Only the top document talks to the server. Sealed guest frames have
    // opaque origins and relay their fetches through it.
    if (location.origin !== "__TONK_ORIGIN__") return;

    const nativeFetch = globalThis.fetch.bind(globalThis);

    const isLiveQuery = (request, url) =>
        url.origin === location.origin &&
        url.pathname.startsWith("/api/") &&
        (request.headers.get("accept") || "").includes("text/event-stream");

    globalThis.fetch = function fetch(input, init) {
        let request;
        try {
            request = new Request(input, init);
        } catch {
            return nativeFetch(input, init);
        }
        const url = new URL(request.url);
        if (!isLiveQuery(request, url)) return nativeFetch(input, init);
        return request.text().then((body) => stream(request, url, body));
    };

    // Send `request` over a WebSocket. The server answers with one text
    // message, the response head, then binary messages with the body, and
    // closes when the body ends.
    function stream(request, url, body) {
        return new Promise((resolve, reject) => {
            const signal = request.signal;
            if (signal.aborted) {
                reject(new DOMException("The request was aborted", "AbortError"));
                return;
            }
            const socket = new WebSocket(`ws://${location.host}/__tonk/stream`);
            socket.binaryType = "arraybuffer";
            let bodyController = null;
            let settled = false;

            const abort = () => {
                const error = new DOMException("The request was aborted", "AbortError");
                if (bodyController) {
                    try {
                        bodyController.error(error);
                    } catch {}
                } else if (!settled) {
                    settled = true;
                    reject(error);
                }
                socket.close();
            };
            signal.addEventListener("abort", abort, { once: true });

            socket.onopen = () => {
                socket.send(
                    JSON.stringify({
                        method: request.method,
                        path: url.pathname + url.search,
                        headers: [...request.headers],
                        body,
                    }),
                );
            };
            socket.onmessage = (event) => {
                if (!settled) {
                    settled = true;
                    const head = JSON.parse(event.data);
                    const responseBody = new ReadableStream({
                        start(controller) {
                            bodyController = controller;
                        },
                        cancel() {
                            socket.close();
                        },
                    });
                    resolve(
                        new Response(responseBody, {
                            status: head.status,
                            headers: head.headers,
                        }),
                    );
                    return;
                }
                if (bodyController && event.data instanceof ArrayBuffer) {
                    try {
                        bodyController.enqueue(new Uint8Array(event.data));
                    } catch {}
                }
            };
            socket.onclose = () => {
                signal.removeEventListener("abort", abort);
                if (!settled) {
                    settled = true;
                    reject(new TypeError("the live query stream closed before it answered"));
                } else if (bodyController) {
                    try {
                        bodyController.close();
                    } catch {}
                }
            };
        });
    }
})();
