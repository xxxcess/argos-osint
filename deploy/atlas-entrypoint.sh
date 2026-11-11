#!/bin/sh
set -eu

role=$1
shift

if [ "$role" = atlas ] && [ -n "${FLEETWORKS_BROKER_URL_FILE:-}" ]; then
    unset FLEETWORKS_BROKER_URL
    exec "$@"
fi

secret=/run/secrets/broker_password

invalid_secret() {
    printf '%s\n' 'RabbitMQ requires an owner-only broker password file with 32–128 ASCII letters, digits, or -._~.' >&2
    exit 1
}

[ -f "$secret" ] || invalid_secret
case "$(stat -c '%a' "$secret")" in
    400|600) ;;
    *) invalid_secret ;;
esac
[ "$(wc -c < "$secret")" -le 130 ] || invalid_secret
password=$(cat "$secret")
password=${password%"$(printf '\r')"}
[ "${#password}" -ge 32 ] && [ "${#password}" -le 128 ] || invalid_secret
case "$password" in
    *[!A-Za-z0-9._~-]*) invalid_secret ;;
esac

umask 077
private=$(mktemp -d /tmp/fleetworks-broker.XXXXXXXX)
case "$role" in
    rabbitmq)
        RABBITMQ_CONFIG_FILE=$private/rabbitmq.conf
        printf '%s\n' \
            'default_user = fleetworks' \
            "default_pass = $password" \
            'default_vhost = fleetworks' \
            'default_user_tags.administrator = false' \
            'log.console = true' \
            'log.file = false' > "$RABBITMQ_CONFIG_FILE"
        unset RABBITMQ_CONFIG_FILES
        export RABBITMQ_CONFIG_FILE
        ;;
    atlas)
        FLEETWORKS_BROKER_URL_FILE=$private/amqp-url
        printf 'amqp://fleetworks:%s@rabbitmq:5672/fleetworks\n' "$password" > "$FLEETWORKS_BROKER_URL_FILE"
        unset FLEETWORKS_BROKER_URL
        export FLEETWORKS_BROKER_URL_FILE
        ;;
    *) exit 1 ;;
esac
unset password
exec "$@"
