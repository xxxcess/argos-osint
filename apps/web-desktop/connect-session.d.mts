import type { RegisteredWebContext } from "@argos/api-client";

export const CONNECT_COOKIE: string;
export const EXPECTED_CONTEXT_HEADER: string;
export const CONNECT_RESPONSE_HEADERS: Readonly<Record<string, string>>;
export interface ConnectRequest {
  headers:
    | { get(name: string): string | null }
    | Record<string, string | string[] | undefined>;
  url?: string;
  method?: string;
}
export interface ServerConnection {
  token: string;
  context: RegisteredWebContext;
  expires_at: string;
  origin: string;
}
export class ConnectFailure extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string);
}
export function connectFailureResponse(error: unknown): Response;
export function headerValue(
  headers: ConnectRequest["headers"],
  name: string,
): string | null | undefined;
export function safeReturnPath(raw: unknown): string;
export function isPublicWebPath(pathname: string): boolean;
export function relativeWebPath(pathname: string): string | undefined;
export function requireWebBoundary(
  request: ConnectRequest,
  mutation?: boolean,
): Promise<string>;
export function requireDevelopmentUpgrade(
  request: ConnectRequest,
  development: boolean,
  path: string | undefined,
): Promise<void>;
export function sessionTokenFromCookie(raw: unknown): string;
export function requireConnection(
  request: ConnectRequest,
  options?: { mutation?: boolean; expectedRevision?: number },
): Promise<ServerConnection>;
export function connectionProofHeaders(
  session: ServerConnection,
): Record<string, string>;
export function publicConnection(session: ServerConnection): {
  context: RegisteredWebContext;
  expires_at: string;
};
export function createConnectionResponse(request: Request): Promise<Response>;
export function currentConnectionResponse(request: Request): Promise<Response>;
export function logoutConnectionResponse(request: Request): Promise<Response>;
export function createConnectAuthority(options?: {
  request?: (path: string, body: unknown) => Promise<unknown>;
  now?: () => number;
}): {
  create(connectId: string): Promise<Omit<ServerConnection, "origin">>;
  resolve(
    token: string,
    expectedRevision?: number,
  ): Promise<Omit<ServerConnection, "origin">>;
  logout(token: string): Promise<void>;
};
export const connectAuthority: ReturnType<typeof createConnectAuthority>;
