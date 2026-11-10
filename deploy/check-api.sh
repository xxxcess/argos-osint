#!/bin/sh
set -eu

token=$(head -c 259 "$FLEETWORKS_API_TOKEN_FILE")
token=${token%"$(printf '\r')"}
[ "${#token}" -ge 32 ] && [ "${#token}" -le 256 ] || exit 1
case "$token" in
    *[!A-Za-z0-9._~+/=-]*) exit 1 ;;
esac

# Feed credentials over stdin so they are absent from the curl argument list.
code=$(printf 'header = "Authorization: Bearer %s"\nheader = "Host: %s"\n' "$token" "$FLEETWORKS_API_HEALTH_HOST" |
    curl -q --config - --noproxy '*' --silent --max-time 3 --output /dev/null \
        --write-out '%{http_code}' http://127.0.0.1:8080/api/v1/health/ready)
[ "$code" = 200 ]
