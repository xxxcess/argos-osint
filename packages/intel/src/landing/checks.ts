import type { Health } from "@argos/api-client";
export type ConnectionCheck = { id: string; label: string; passed: boolean };
/** Checks report the existing readiness response; no staged or fabricated probes. */
export function connectionChecks(health?: Health): ConnectionCheck[] {
  return [
    { id: "api", label: "Local API reachable", passed: !!health },
    {
      id: "service",
      label: "Intel service identified",
      passed: health?.service === "argos-intel",
    },
    {
      id: "protocol",
      label: "API compatible",
      passed: health?.api_version === 1,
    },
    { id: "storage", label: "Storage ready", passed: health?.storage === true },
    {
      id: "workspace",
      label: "Workspace readiness passed",
      passed: health?.ready === true,
    },
  ];
}
