const association = JSON.stringify({
  webcredentials: { apps: ["8WVKS2F24C.xyz.tonk"] },
});

// Temporary spike route. Remove after the normal Tonk release serves the asset.
// No bindings, secrets, authentication requests, or account data are handled here.
export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.hostname !== "tonk.network" ||
        url.pathname !== "/.well-known/apple-app-site-association") {
      return new Response("Not found", { status: 404 });
    }
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response(null, { status: 405, headers: { Allow: "GET, HEAD" } });
    }
    return new Response(request.method === "HEAD" ? null : association, {
      headers: {
        "Content-Type": "application/json",
        "Cache-Control": "public, max-age=300",
      },
    });
  },
};
