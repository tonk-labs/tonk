"""Validate an Apple profile and extract the desktop passkey entitlements."""
import datetime
import hashlib
import os
import plistlib
import sys
from pathlib import Path

TEAM = "8WVKS2F24C"
# Each channel's app identifier and the domain its passkeys belong to.
CHANNELS = {
    "stable": ("xyz.tonk", "webcredentials:tonk.network"),
    "nightly": ("xyz.tonk.nightly", "webcredentials:tonk.foundation"),
}


def entitlements_for(profile, identity=None, now=None, channel="stable"):
    if channel not in CHANNELS:
        raise ValueError(f"Unknown desktop channel: {channel}")
    bundle_id, domain = CHANNELS[channel]
    entitlements = profile["Entitlements"]
    if entitlements.get("com.apple.application-identifier") != TEAM + "." + bundle_id:
        raise ValueError(f"Passkey builds require the Tonk Labs {bundle_id} profile.")
    if entitlements.get("com.apple.developer.team-identifier") != TEAM:
        raise ValueError("The provisioning profile belongs to a different team.")
    now = now or datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
    if profile["ExpirationDate"] <= now:
        raise ValueError("The passkey provisioning profile has expired.")
    domains = entitlements.get("com.apple.developer.associated-domains", [])
    if "*" not in domains and domain not in domains:
        raise ValueError("The provisioning profile does not allow the Tonk domain.")
    if identity is not None:
        allowed = {hashlib.sha1(cert).hexdigest().upper() for cert in profile.get("DeveloperCertificates", [])}
        if identity.upper() not in allowed:
            raise ValueError("The provisioning profile does not authorize the signing certificate.")
    return {
        "com.apple.application-identifier": TEAM + "." + bundle_id,
        "com.apple.developer.team-identifier": TEAM,
        "com.apple.developer.associated-domains": [domain],
    }


if __name__ == "__main__":
    try:
        profile = plistlib.loads(Path(sys.argv[1]).read_bytes())
        selected = entitlements_for(profile, sys.argv[3] if len(sys.argv) > 3 else None,
                                    channel=os.environ.get("TONK_DESKTOP_CHANNEL", "stable"))
        Path(sys.argv[2]).write_bytes(plistlib.dumps(selected))
    except (ValueError, KeyError) as error:
        raise SystemExit(str(error)) from error
