// Routes the subdomains of the dev zone (`*.tonk.foundation`).
//
// A pull request's preview is a version of the preview worker, reached at an
// alias on `workers.dev`. Nothing can be served beneath that hostname, and a
// version has no routes of its own, so a preview has nowhere to put the
// origins its sites render on. This worker gives it names in the dev zone:
//
// - `pr-33.tonk.foundation` is the preview's app.
// - `profile-pr33.tonk.foundation` and `{space}-pr33.tonk.foundation` are its
//   sites. A level of their own (`{space}.pr-33.…`) is past what a wildcard
//   certificate covers, so the preview is named within the label.
//
// Both are answered by that pull request's preview. Every other subdomain is
// one of the dev deployment's own sites and goes to the dev worker.

// A space's label is base32 and the profile's is `profile`: neither has a
// `-`, so a `-pr{n}` tail can only be a preview's.
const APP = /^pr-(\d+)$/;
const SITE = /^[a-z0-9]+-pr(\d+)$/;

/// The pull request a hostname belongs to, or `null` for the dev deployment's.
export function previewOf(hostname) {
    const label = hostname.split(".")[0];
    const match = APP.exec(label) ?? SITE.exec(label);
    return match ? Number(match[1]) : null;
}

/// Where pull request `number`'s preview answers.
export function previewOrigin(number, env) {
    return env.PREVIEW_ORIGIN.replace("{number}", String(number));
}

export default {
    async fetch(request, env) {
        const url = new URL(request.url);
        const number = previewOf(url.hostname);
        if (number == null) return env.DEV.fetch(request);

        const target = new URL(url.pathname + url.search, previewOrigin(number, env));
        const forwarded = new Request(target, request);
        // The preview sees its own `workers.dev` name as the host. It is
        // told the name the browser asked for, which decides whether its
        // configuration names origins for sites.
        forwarded.headers.set("x-forwarded-host", url.host);
        // A redirect is the browser's to follow, against the name it asked
        // for: an invite's short link carries its fragment across one.
        return fetch(forwarded, { redirect: "manual" });
    },
};
