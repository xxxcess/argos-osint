import type { Server, IncomingMessage, ServerResponse } from "node:http";
import type { RegisteredWebContext } from "@argos/api-client";
export function startConnectGateway(options?: {
  apiToken?: string;
  dataHandler?: (
    request: IncomingMessage,
    response: ServerResponse,
    body: unknown,
  ) => Promise<boolean> | boolean;
}): Promise<{
  server: Server;
  origin: string;
  apiToken: string;
  sessionToken: string;
  context: RegisteredWebContext;
  received: {
    path: string;
    method: string;
    headers: Record<string, string | string[] | undefined>;
    body: unknown;
  }[];
  sessions: Map<string, RegisteredWebContext>;
  revoked: Set<string>;
  setUnavailable(value: boolean): void;
  close(): Promise<void>;
}>;
export function installGatewayEnvironment(
  gateway: { origin: string; apiToken: string },
  webOrigin?: string,
): () => void;
