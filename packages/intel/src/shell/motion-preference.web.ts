const query = "(prefers-reduced-motion: reduce)";
export function reducedMotionSnapshot() {
  return typeof window === "undefined" || !window.matchMedia
    ? true
    : window.matchMedia(query).matches;
}
export function subscribeReducedMotion(listener: () => void) {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const media = window.matchMedia(query);
  media.addEventListener("change", listener);
  return () => media.removeEventListener("change", listener);
}
