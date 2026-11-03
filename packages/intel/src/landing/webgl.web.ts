let supported: boolean | undefined;
/** Capability probe once per page; release its temporary GPU context immediately. */
export function webGL() {
  if (supported !== undefined) return supported;
  if (
    typeof document === "undefined" ||
    typeof WebGL2RenderingContext === "undefined"
  )
    return false;
  try {
    const context = document.createElement("canvas").getContext("webgl2");
    supported = !!context;
    context?.getExtension("WEBGL_lose_context")?.loseContext();
  } catch {
    supported = false;
  }
  return supported;
}
