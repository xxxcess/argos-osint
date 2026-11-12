# Linux server images for the web app, Venture, and each Fleetworks CLI.
# The client targets share clients_build, so one compile feeds every selected image.
#
# Bake resolves local contexts from the working directory. The documented
# command runs at the repository root, so cwd://. is that checkout. A parent
# path leaves the checkout and current Buildx rejects the read.

variable "PLATFORM" {
  default = "linux/amd64"
}

# Workspace SemVer from version.txt, with no leading v. CI and the deploy
# guide export VERSION from that file. :local remains the development alias.
variable "VERSION" {}

# Explicit review tags never replace the aliases selected by an existing stack.
variable "IMAGE_TAG" {
  default = ""
}

group "backend" {
  targets = ["runtime", "surrealdb", "search-index"]
}

target "search-index" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "search-index"
  tags       = IMAGE_TAG != "" ? ["fleetworks-search-index:${IMAGE_TAG}"] : ["fleetworks-search-index:local", "fleetworks-search-index:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "runtime" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "runtime"
  tags       = IMAGE_TAG != "" ? ["fleetworks-runtime:${IMAGE_TAG}"] : ["fleetworks-runtime:local", "fleetworks-runtime:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "surrealdb" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "surrealdb"
  tags       = IMAGE_TAG != "" ? ["fleetworks-surrealdb:${IMAGE_TAG}"] : ["fleetworks-surrealdb:local", "fleetworks-surrealdb:${VERSION}"]
  platforms  = ["${PLATFORM}"]
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
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "web"
  tags       = IMAGE_TAG != "" ? ["web:${IMAGE_TAG}"] : ["web:local", "web:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "venture" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "venture"
  tags       = IMAGE_TAG != "" ? ["venture:${IMAGE_TAG}"] : ["venture:local", "venture:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-atlas" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-atlas"
  tags       = IMAGE_TAG != "" ? ["fleetworks-atlas:${IMAGE_TAG}"] : ["fleetworks-atlas:local", "fleetworks-atlas:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-brain" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-brain"
  tags       = IMAGE_TAG != "" ? ["fleetworks-brain:${IMAGE_TAG}"] : ["fleetworks-brain:local", "fleetworks-brain:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-jobs" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-jobs"
  tags       = IMAGE_TAG != "" ? ["fleetworks-jobs:${IMAGE_TAG}"] : ["fleetworks-jobs:local", "fleetworks-jobs:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-tools" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-tools"
  tags       = IMAGE_TAG != "" ? ["fleetworks-tools:${IMAGE_TAG}"] : ["fleetworks-tools:local", "fleetworks-tools:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-models" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-models"
  tags       = IMAGE_TAG != "" ? ["fleetworks-models:${IMAGE_TAG}"] : ["fleetworks-models:local", "fleetworks-models:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}

target "fleetworks-profile" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "fleetworks-profile"
  tags       = IMAGE_TAG != "" ? ["fleetworks-profile:${IMAGE_TAG}"] : ["fleetworks-profile:local", "fleetworks-profile:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}
