"use client";
import { BackendProvider } from "@argos/api-client";
import { IntelApp } from "@argos/intel";
import { webBackendSettings } from "./backend-settings";
export default function Page() {
  return (
    <BackendProvider settings={webBackendSettings}>
      <IntelApp />
    </BackendProvider>
  );
}
