/** Non-secret server-resolved context. Browser values never grant authority. */
export interface RegisteredWebContext {
  deployment_id: string;
  registration_id: string;
  workstation_id: string;
  workstation_ip?: string | null;
  connect_id: string;
  registration_revision: number;
  context_revision: number;
  scope_revision: number;
  configuration_epoch: string;
  access_epoch: string;
}

/** Per-mounted-session request fence; it contains no bearer/session credential. */
export interface ConnectionRequestScope {
  context: RegisteredWebContext;
  onInvalidated?: () => void;
}

export function connectionNamespace(context?: RegisteredWebContext): string {
  return context
    ? JSON.stringify([
        "workstation-v2",
        context.deployment_id,
        context.access_epoch,
        context.workstation_id,
        context.registration_id,
        context.registration_revision,
        context.context_revision,
        context.scope_revision,
        context.configuration_epoch,
      ])
    : "";
}
