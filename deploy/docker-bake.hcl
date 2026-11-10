# Linux server images for the web app, Venture, and each Fleetworks CLI.
# The client targets share clients_build, so one compile feeds every selected image.

variable "PLATFORM" {
  default = "linux/amd64"
}

group "clients" {
  targets = [
    "venture",
    "fleetworks-atlas",
    "fleetworks-brain",
    "fleetworks-jobs",
    "fleetworks-tools",
    "fleetworks-models",
    "fleetworks-profile",
  ]
}

group "server" {
  targets = ["web", "venture", "fleetworks-atlas", "fleetworks-brain", "fleetworks-jobs", "fleetworks-tools", "fleetworks-models", "fleetworks-profile"]
}

target "web" {
  context    = ".."
  dockerfile = "deploy/Dockerfile"
  target     = "web"
  tags       = ["web:local"]
  platforms  = ["${PLATFORM}"]
}

target "venture" {
  context    = ".."
  dockerfile = "deploy/Dockerfile"
  target     = "venture"
  tags       = ["venture:local"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-atlas" {
  context    = ".."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-atlas"
  tags       = ["fleetworks-atlas:local"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-brain" {
  context    = ".."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-brain"
  tags       = ["fleetworks-brain:local"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-jobs" {
  context    = ".."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-jobs"
  tags       = ["fleetworks-jobs:local"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-tools" {
  context    = ".."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-tools"
  tags       = ["fleetworks-tools:local"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-models" {
  context    = ".."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-models"
  tags       = ["fleetworks-models:local"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-profile" {
  context    = ".."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-profile"
  tags       = ["fleetworks-profile:local"]
  platforms  = ["${PLATFORM}"]
}
