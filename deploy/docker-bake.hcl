# Linux backend images plus the optional Intel Web service.
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

group "server" {
  targets = ["runtime", "surrealdb", "search-index", "web"]
}

target "web" {
  context    = "cwd://."
  dockerfile = "deploy/Dockerfile"
  target     = "web"
  tags       = IMAGE_TAG != "" ? ["fleetworks-web:${IMAGE_TAG}"] : ["fleetworks-web:local", "fleetworks-web:${VERSION}"]
  platforms  = ["${PLATFORM}"]
}
