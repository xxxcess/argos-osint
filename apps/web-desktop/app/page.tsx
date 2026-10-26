"use client";
import { BackendProvider } from "@argos/api-client";
import { IntelApp } from "@argos/intel";
export default function Page() {
  return (
    <BackendProvider>
      <IntelApp />
    </BackendProvider>
  );
}
