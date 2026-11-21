# The packaging helper stages only this file, the Dockerfile and the verified
# native bundle in the BuildKit context.
target "native-embedded" {
  context    = "cwd://."
  dockerfile = "Dockerfile.native-embedded"
  platforms  = ["linux/amd64"]
}
