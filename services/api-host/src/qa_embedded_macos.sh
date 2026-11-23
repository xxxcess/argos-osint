# The pinned Intel OpenMP runtime registers one POSIX object named for its process.
# exec retains the shell PID in sandbox-exec and then the isolated model worker.
case "$1" in
    x86_64) omp_name="/__KMP_REGISTERED_LIB_$$" ;;
    aarch64) omp_name="" ;;
    *) exit 2 ;;
esac
shift
umask 077
exec /usr/bin/sandbox-exec -D "omp_name=$omp_name" "$@"
