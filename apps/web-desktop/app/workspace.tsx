"use client";
import { BackendProvider } from "@argos/api-client";
import { IntelApp } from "@argos/intel";
import { webBackendSettings } from "./backend-settings";
import { webNavigation } from "./navigation";
import { webLiveTransport } from "./live-transport";
import { WEB_UI_SCALE } from "./ui-scale";

/** Persistent shell across the four screen routes; history never resets feed state. */
export function WebWorkspace() {
  return (
    <BackendProvider settings={webBackendSettings} live={webLiveTransport}>
      <IntelApp navigation={webNavigation} displayScale={WEB_UI_SCALE} />
    </BackendProvider>
  );
}
