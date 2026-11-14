/** Browser-only notification; messages contain no credential, ID or context data. */
const channelName = "fleetworks-connect-context";

export function announceConnectionChange() {
  if (typeof BroadcastChannel === "undefined") return;
  const channel = new BroadcastChannel(channelName);
  channel.postMessage("context-changed");
  channel.close();
}

export function subscribeConnectionChanges(listener: () => void) {
  if (typeof BroadcastChannel === "undefined") return () => {};
  const channel = new BroadcastChannel(channelName);
  channel.onmessage = (message) => {
    if (message.data === "context-changed") listener();
  };
  return () => channel.close();
}
