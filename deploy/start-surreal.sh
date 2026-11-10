#!/bin/sh
set -eu

secret_file=$FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE
[ "$(/fleetworks-busybox wc -c < "$secret_file")" -le 4096 ] || exit 1
SURREAL_USER=$FLEETWORKS_STORAGE_ROOT_USERNAME
SURREAL_PASS=$(/fleetworks-busybox cat "$secret_file")
SURREAL_PASS=${SURREAL_PASS%"$(printf '\r')"}
[ -n "$SURREAL_PASS" ] || exit 1
export SURREAL_USER SURREAL_PASS

exec /surreal start --no-banner --bind 0.0.0.0:8000 'rocksdb:///data/store?versioned=false&sync=every'
