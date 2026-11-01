import { useEffect } from "react";
/** Scroll the list twin only; selecting a canvas node must not scroll the page. */
export function useSelectedNodeScroll(id: string | undefined) {
  useEffect(() => {
    if (!id) return;
    const row = document.getElementById(`intel-node-${id}`),
      list = document.getElementById("intel-topology-node-scroll");
    if (!row || !list) return;
    const rect = row.getBoundingClientRect(),
      viewport = list.getBoundingClientRect();
    if (rect.top < viewport.top) list.scrollTop += rect.top - viewport.top;
    else if (rect.bottom > viewport.bottom)
      list.scrollTop += rect.bottom - viewport.bottom;
  }, [id]);
}
