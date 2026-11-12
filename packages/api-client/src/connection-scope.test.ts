import { describe, expect, it } from "vitest";
import { connectionNamespace } from "./connection-scope";
import type { RegisteredWebContext } from "./connection-scope";

const context: RegisteredWebContext = {
  deployment_id: "deployment-a",
  registration_id: "registration-a",
  workstation_id: "node-a",
  workstation_ip: "192.0.2.42",
  workspace_id: "workspace-a",
  workspace_name: "research",
  connect_id: "192.0.2.42:research",
  registration_revision: 1,
  context_revision: 7,
  scope_revision: 2,
  configuration_epoch: "reset-1",
  access_epoch: "access-1",
};

describe("registered browser cache scope", () => {
  it("isolates deployments, access, workspaces and independent context/scope epochs", () => {
    const original = connectionNamespace(context);
    expect(connectionNamespace({ ...context })).toBe(original);
    for (const change of [
      { deployment_id: "deployment-b" },
      { access_epoch: "access-2" },
      { workspace_id: "workspace-b" },
      { registration_id: "registration-b" },
      { registration_revision: 2 },
      { context_revision: 8 },
      { scope_revision: 3 },
      { configuration_epoch: "reset-2" },
    ])
      expect(connectionNamespace({ ...context, ...change })).not.toBe(original);
    expect(connectionNamespace()).toBe("");
    expect(original).not.toContain("token");
  });
});
