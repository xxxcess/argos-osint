#!/bin/sh
set -eu

# Bootstrap exports the retained DB identity into a separate read-only mount.
# Missing/mismatching identities fail; startup never resets index generations.
if [ -n "${FLEETWORKS_DEPLOYMENT_ID_FILE:-}" ]; then
    [ -f "$FLEETWORKS_DEPLOYMENT_ID_FILE" ] && [ ! -L "$FLEETWORKS_DEPLOYMENT_ID_FILE" ] || exit 1
    [ "$(wc -c < "$FLEETWORKS_DEPLOYMENT_ID_FILE")" -le 257 ] || exit 1
    deployment=$(cat "$FLEETWORKS_DEPLOYMENT_ID_FILE")
    case "$deployment" in ''|*[!A-Za-z0-9._:-]*) exit 1 ;; esac
    [ "${#deployment}" -le 256 ] || exit 1
    if [ -n "${FLEETWORKS_SEARCH_DEPLOYMENT_ID:-}" ]; then
        [ "$deployment" = "$FLEETWORKS_SEARCH_DEPLOYMENT_ID" ] || exit 1
    fi
    FLEETWORKS_SEARCH_DEPLOYMENT_ID=$deployment
    export FLEETWORKS_SEARCH_DEPLOYMENT_ID
fi
exec /usr/local/bin/fleetworks-search-index
