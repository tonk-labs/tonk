# Desktop build channel, sourced by the build, signing and notarization scripts.
# Stable is xyz.tonk on tonk.network; nightly is a separate app on tonk.foundation,
# so both can be installed side by side. Only a published channel has an update feed.
desktop_channel=${TONK_DESKTOP_CHANNEL:-stable}
case "$desktop_channel" in
    stable) desktop_app_name=Tonk; desktop_bundle_id=xyz.tonk; desktop_feed_url= ;;
    nightly) desktop_app_name="Tonk Nightly"; desktop_bundle_id=xyz.tonk.nightly
        desktop_feed_url=https://github.com/tonk-labs/tonk/releases/download/tonk-desktop-dev/Tonk-appcast.xml ;;
    *) echo "TONK_DESKTOP_CHANNEL must be stable or nightly, not $desktop_channel." >&2; exit 1 ;;
esac
