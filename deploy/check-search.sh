#!/bin/sh
set -eu

token=$(head -c 259 "$FLEETWORKS_SEARCH_TOKEN_FILE")
token=${token%"$(printf '\r')"}
[ "${#token}" -ge 32 ] && [ "${#token}" -le 256 ] || exit 1
case "$token" in *[!A-Za-z0-9._~+/=-]*) exit 1 ;; esac
host=${FLEETWORKS_SEARCH_HEALTH_HOST:-search-index:8087}
case "$host" in ''|*[!A-Za-z0-9.:-]*) exit 1 ;; esac
# A healthy empty/not-built generation is still a functioning private service.
# The body carries discovery readiness; health never seeds or repairs an index.
code=$(printf 'header = "Authorization: Bearer %s"\nheader = "Host: %s"\n' "$token" "$host" |
    curl -q --config - --noproxy '*' --silent --max-time 3 --output /dev/null \
        --write-out '%{http_code}' http://127.0.0.1:8087/internal/v1/actors/status)
[ "$code" = 200 ]
