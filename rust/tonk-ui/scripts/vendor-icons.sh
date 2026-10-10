#!/bin/sh
# Vendor the Font Awesome Free icons `<wa-icon>` reads into the bundle.
#
# Web Awesome fetches an icon as `{icon path}/{family folder}/{name}.svg`, and
# by default from Font Awesome's own host. Nothing the app shows is fetched
# from a third party, so the free set is committed under
# `assets/webawesome/icons` and every page points Web Awesome's icon path at
# it (see `runtime_bootstrap.js`). A site's pages reach nothing but their own
# origin, which serves these.
#
# The release has to be the one the vendored Web Awesome asks for (`FA_VERSION`
# in its icon chunk): the names it knows are that release's. Re-run this after
# a Web Awesome bump and commit the result:
#
#   sh scripts/vendor-icons.sh
#
# Only the families the free set has (`solid`, `regular`, `brands`); an icon
# of another family is one the app does not ship.
set -eu

ASSETS="$(cd "$(dirname "$0")/../assets/webawesome" && pwd)"
VERSION=$(sed -n 's/^var FA_VERSION = "\(.*\)";$/\1/p' "$ASSETS"/chunks/*.js | head -n 1)
[ -n "$VERSION" ] || {
    echo "vendor-icons: the vendored Web Awesome names no icon release" >&2
    exit 1
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
(cd "$WORK" && npm pack --silent "@fortawesome/fontawesome-free@$VERSION" > /dev/null)
tar -xzf "$WORK"/fortawesome-fontawesome-free-"$VERSION".tgz -C "$WORK"

rm -rf "$ASSETS/icons"
mkdir -p "$ASSETS/icons"
for FAMILY in solid regular brands; do
    cp -R "$WORK/package/svgs/$FAMILY" "$ASSETS/icons/$FAMILY"
done
cp "$WORK/package/LICENSE.txt" "$ASSETS/icons/LICENSE.txt"
printf '%s\n' "$VERSION" > "$ASSETS/icons/VERSION"
echo "vendor-icons: Font Awesome Free $VERSION, $(find "$ASSETS/icons" -name '*.svg' | wc -l | tr -d ' ') icons"
