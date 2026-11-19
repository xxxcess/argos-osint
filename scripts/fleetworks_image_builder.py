"""Construct native image build commands for CI and development packaging."""


def bake_command(bakefile, platform, targets, settings=(), *, builder="buildx", project=None):
    """Build an explicit target selection and load it into the local Docker daemon."""
    if builder == "buildx":
        command = ("docker", "buildx", "bake")
    elif builder == "depot":
        if not project:
            raise ValueError("Depot Bake requires an explicit project")
        command = ("depot", "bake", "--project", project, "--build-platform", platform)
    else:
        raise ValueError(f"Unsupported image builder: {builder}")
    return (*command, "-f", str(bakefile), "--load", "--set", f"*.platforms={platform}",
            *settings, *targets)
